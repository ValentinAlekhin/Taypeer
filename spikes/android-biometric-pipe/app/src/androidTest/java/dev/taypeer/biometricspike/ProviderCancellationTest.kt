package dev.taypeer.biometricspike

import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.taypeer.platform.BiometricKeyPipe
import java.nio.ByteBuffer
import java.security.AlgorithmParameters
import java.security.Key
import java.security.KeyPairGenerator
import java.security.Provider
import java.security.SecureRandom
import java.security.spec.AlgorithmParameterSpec
import java.security.spec.MGF1ParameterSpec
import java.security.spec.RSAKeyGenParameterSpec
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.CipherSpi
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource
import javax.crypto.spec.SecretKeySpec
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** The gate delays the real PUBLIC JCA operation, never inventing a decrypt result. */
@RunWith(AndroidJUnit4::class)
class ProviderCancellationTest {
    @Test(timeout = 15_000) fun cancellationClosesThePipeBeforeABlockedProviderReturnsAndLateDecryptCannotPublish() {
        val parameters = OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT)
        val keys = KeyPairGenerator.getInstance("RSA").apply {
            initialize(RSAKeyGenParameterSpec(2048, RSAKeyGenParameterSpec.F4))
        }.generateKeyPair()
        val ciphertext = Cipher.getInstance("RSA/ECB/OAEPWithSHA-256AndMGF1Padding").apply {
            init(Cipher.ENCRYPT_MODE, keys.public, parameters)
        }.doFinal(ByteArray(32) { 0x50 })
        val real = Cipher.getInstance("RSA/ECB/OAEPWithSHA-256AndMGF1Padding").apply { init(Cipher.DECRYPT_MODE, keys.private, parameters) }
        val entered = CountDownLatch(1)
        val released = CountDownLatch(1)
        val cipher = GatedCipher(real, entered, released).apply {
            // This PUBLIC dummy only initializes Cipher's state; the gate uses the actual RSA cipher.
            init(Cipher.DECRYPT_MODE, SecretKeySpec(byteArrayOf(0x50), "PUBLIC-gate"))
        }
        val descriptors = ParcelFileDescriptor.createPipe()
        val pipe = BiometricKeyPipe.open(descriptors[1])
        val executor = Executors.newFixedThreadPool(3)
        try {
            val finishing = executor.submit<Boolean> { pipe.finish(cipher, ciphertext) }
            assertTrue("The real provider call must be in flight", entered.await(3, TimeUnit.SECONDS))
            val cancelled = executor.submit { pipe.close() }
            cancelled.get(1, TimeUnit.SECONDS)
            val eof = executor.submit<Int> { ParcelFileDescriptor.AutoCloseInputStream(descriptors[0]).use { it.read() } }
            assertEquals("Cancellation must close the writer while the provider remains blocked", -1, eof.get(1, TimeUnit.SECONDS).toInt())
            assertFalse(finishing.isDone)
            released.countDown()
            assertFalse("A late real decrypt must not publish to a cancelled pipe", finishing.get(3, TimeUnit.SECONDS))
            assertFalse(pipe.finish(cipher, ciphertext))
        } finally {
            released.countDown()
            pipe.close()
            try { descriptors[0].close() } catch (_: java.io.IOException) { }
            executor.shutdownNow()
            assertTrue(executor.awaitTermination(3, TimeUnit.SECONDS))
        }
    }

    private class GatedCipher(real: Cipher, entered: CountDownLatch, released: CountDownLatch) : Cipher(
        Gate(real, entered, released), object : Provider("PUBLIC-gated-provider", 1.0, "PUBLIC test gate") {}, "RSA/ECB/OAEPPadding",
    )

    private class Gate(private val real: Cipher, private val entered: CountDownLatch, private val released: CountDownLatch) : CipherSpi() {
        override fun engineSetMode(mode: String) = Unit
        override fun engineSetPadding(padding: String) = Unit
        override fun engineGetBlockSize(): Int = real.blockSize
        override fun engineGetOutputSize(size: Int): Int = real.getOutputSize(size)
        override fun engineGetIV(): ByteArray? = real.iv
        override fun engineGetParameters(): AlgorithmParameters? = real.parameters
        override fun engineInit(mode: Int, key: Key, random: SecureRandom?) = Unit
        override fun engineInit(mode: Int, key: Key, parameters: AlgorithmParameterSpec?, random: SecureRandom?) = Unit
        override fun engineInit(mode: Int, key: Key, parameters: AlgorithmParameters?, random: SecureRandom?) = Unit
        override fun engineUpdate(input: ByteArray, offset: Int, length: Int): ByteArray = error("PUBLIC gate requires ByteBuffer API")
        override fun engineUpdate(input: ByteArray, offset: Int, length: Int, output: ByteArray, outputOffset: Int): Int = error("PUBLIC gate requires ByteBuffer API")
        override fun engineDoFinal(input: ByteArray, offset: Int, length: Int): ByteArray = error("PUBLIC gate requires ByteBuffer API")
        override fun engineDoFinal(input: ByteArray, offset: Int, length: Int, output: ByteArray, outputOffset: Int): Int = error("PUBLIC gate requires ByteBuffer API")
        override fun engineDoFinal(input: ByteBuffer, output: ByteBuffer): Int {
            entered.countDown()
            check(released.await(5, TimeUnit.SECONDS)) { "PUBLIC provider gate timed out" }
            return real.doFinal(input, output)
        }
    }
}
