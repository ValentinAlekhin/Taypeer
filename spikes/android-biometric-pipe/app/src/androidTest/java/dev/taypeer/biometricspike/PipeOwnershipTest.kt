package dev.taypeer.biometricspike

import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.taypeer.platform.BiometricKeyPipe
import java.io.File
import java.io.IOException
import java.security.KeyPairGenerator
import java.security.spec.MGF1ParameterSpec
import java.security.spec.RSAKeyGenParameterSpec
import javax.crypto.Cipher
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Actual JCA, product JNI and real pipe tests. These do not claim biometric authorization. */
@RunWith(AndroidJUnit4::class)
class PipeOwnershipTest {
    private val oaepParameters = OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT)
    private fun cipherFixture(length: Int): Pair<Cipher, ByteArray> {
        val pair = KeyPairGenerator.getInstance("RSA").apply {
            initialize(RSAKeyGenParameterSpec(2048, RSAKeyGenParameterSpec.F4))
        }.generateKeyPair()
        val encrypted = Cipher.getInstance("RSA/ECB/OAEPWithSHA-256AndMGF1Padding").apply {
            init(Cipher.ENCRYPT_MODE, pair.public, oaepParameters)
        }.doFinal(ByteArray(length) { 0x50 }) // Intentionally PUBLIC patterned bytes.
        val decrypt = Cipher.getInstance("RSA/ECB/OAEPWithSHA-256AndMGF1Padding").apply { init(Cipher.DECRYPT_MODE, pair.private, oaepParameters) }
        return decrypt to encrypted
    }

    private fun readBounded(input: ParcelFileDescriptor): ByteArray =
        ParcelFileDescriptor.AutoCloseInputStream(input).use { stream ->
            val output = ByteArray(33)
            var count = 0
            while (count < output.size) {
                val received = stream.read(output, count, output.size - count)
                if (received < 0) break
                count += received
            }
            assertTrue("The one-shot key pipe must end after at most 32 bytes", count <= 32)
            output.copyOf(count)
        }

    @Test fun realRsaDecryptWritesExactly32BytesAndEofOnlyOnce() {
        val (cipher, wrapped) = cipherFixture(32)
        val descriptors = ParcelFileDescriptor.createPipe()
        BiometricKeyPipe.open(descriptors[1]).use { pipe ->
            assertTrue(pipe.finish(cipher, wrapped))
            assertArrayEquals(ByteArray(32) { 0x50 }, readBounded(descriptors[0]))
            assertFalse(pipe.finish(cipher, wrapped))
        }
    }

    @Test fun actual31And33ByteRsaResultsAreRejectedWithoutPublishingAnyKeyBytes() {
        for (length in listOf(31, 33)) {
            val (cipher, wrapped) = cipherFixture(length)
            val descriptors = ParcelFileDescriptor.createPipe()
            BiometricKeyPipe.open(descriptors[1]).use { pipe ->
                assertFalse(pipe.finish(cipher, wrapped))
                assertTrue(readBounded(descriptors[0]).isEmpty())
            }
        }
    }

    @Test fun cancellationClosesTheWriterAndCannotBeConsumedAgain() {
        val (cipher, wrapped) = cipherFixture(32)
        val descriptors = ParcelFileDescriptor.createPipe()
        val pipe = BiometricKeyPipe.open(descriptors[1])
        pipe.close()
        assertTrue(readBounded(descriptors[0]).isEmpty())
        assertFalse(pipe.finish(cipher, wrapped))
        pipe.close()
    }

    @Test fun departedReaderFailsWithoutSigpipeTerminationOrFalsePublicationAck() {
        val (cipher, wrapped) = cipherFixture(32)
        val descriptors = ParcelFileDescriptor.createPipe()
        val pipe = BiometricKeyPipe.open(descriptors[1])
        descriptors[0].close()
        pipe.use { assertFalse(it.finish(cipher, wrapped)) }
        // Reaching this point proves the process survived the real EPIPE write.
        val fresh = ParcelFileDescriptor.createPipe()
        BiometricKeyPipe.open(fresh[1]).close()
        assertTrue(readBounded(fresh[0]).isEmpty())
    }

    @Test fun anOrdinaryFileCannotBecomeAKeyExportDestination() {
        val context = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext
        val file = File.createTempFile("PUBLIC-pipe-rejection", ".fixture", context.cacheDir)
        try {
            file.writeBytes(byteArrayOf(80, 85, 66))
            val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_WRITE_ONLY)
            try { BiometricKeyPipe.open(descriptor); fail("A key sink must be a writable private pipe") }
            catch (_: IOException) { }
            assertArrayEquals(byteArrayOf(80, 85, 66), file.readBytes())
        } finally { assertTrue(file.delete()) }
    }

    @Test fun corruptedOaepCiphertextClosesThePipeWhenTheRealProviderThrows() {
        val (cipher, wrapped) = cipherFixture(32)
        val descriptors = ParcelFileDescriptor.createPipe()
        BiometricKeyPipe.open(descriptors[1]).use { pipe ->
            try { pipe.finish(cipher, wrapped.copyOf().apply { this[0] = (this[0].toInt() xor 1).toByte() }); fail("OAEP authentication must fail") }
            catch (_: javax.crypto.BadPaddingException) { }
            assertTrue(readBounded(descriptors[0]).isEmpty())
        }
    }
}
