package dev.taypeer

import androidx.test.core.app.ApplicationProvider
import android.content.Context
import dev.taypeer.bridge.AndroidException
import dev.taypeer.platform.KeystoreCredentials
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.util.UUID
import org.junit.Assert.*
import org.junit.Test

/** Real Keystore encryption and file publication; only directory ACK is fault injected. */
class CredentialPublicationTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val service = "PUBLIC-credential-publication-${UUID.randomUUID()}"
    private val account = "PUBLIC-account"

    private fun file(selectedAccount: String = account): File {
        val first = service.toByteArray(Charsets.UTF_8)
        val second = selectedAccount.toByteArray(Charsets.UTF_8)
        val address = ByteBuffer.allocate(8 + first.size + second.size)
            .putInt(first.size).put(first).putInt(second.size).put(second).array()
        val digest = MessageDigest.getInstance("SHA-256").digest(address)
        return File(context.noBackupFilesDir, "credentials/" + digest.joinToString("") { "%02x".format(it) } + ".credential")
    }

    @Test fun acknowledgedValueReopensEncryptedAndCallerBytesAreWiped() {
        val expected = "PUBLIC encrypted credential roundtrip".toByteArray()
        val input = expected.copyOf()
        try {
            KeystoreCredentials(context).set(service, account, input)
            assertTrue(input.all { it == 0.toByte() })
            assertArrayEquals(expected, KeystoreCredentials(context).get(service, account))
            assertFalse(file().readBytes().toString(Charsets.ISO_8859_1).contains(String(expected)))
            assertNull(KeystoreCredentials(context).get(service, "PUBLIC missing account"))
        } finally { file().delete(); input.fill(0); expected.fill(0) }
    }

    @Test fun failureAfterActualRenameIsUncertainAndExactRetryMustAcknowledge() {
        val before = "PUBLIC old published value".toByteArray()
        val expected = "PUBLIC new value before lost directory ACK".toByteArray()
        val failedInput = expected.copyOf()
        var directoryCalls = 0
        try {
            KeystoreCredentials(context).set(service, account, before.copyOf())
            val oldCiphertext = file().readBytes()
            val failing = KeystoreCredentials(context) {
                directoryCalls++
                throw IOException("PUBLIC injected directory ACK failure")
            }
            assertThrows(AndroidException.CommitUncertain::class.java) {
                failing.set(service, account, failedInput)
            }
            assertEquals(1, directoryCalls)
            assertTrue(failedInput.all { it == 0.toByte() })
            assertFalse(oldCiphertext.contentEquals(file().readBytes()))
            assertArrayEquals(expected, KeystoreCredentials(context).get(service, account))
            val retry = expected.copyOf()
            KeystoreCredentials(context).set(service, account, retry)
            assertTrue(retry.all { it == 0.toByte() })
            assertArrayEquals(expected, KeystoreCredentials(context).get(service, account))
        } finally { file().delete(); before.fill(0); expected.fill(0); failedInput.fill(0) }
    }

    @Test fun rejectedInputBeforePublicationKeepsPriorCredentialAndWipesInput() {
        val expected = "PUBLIC retained before rejected publication".toByteArray()
        val rejected = ByteArray(65537) { 42 }
        var directoryCalls = 0
        try {
            KeystoreCredentials(context).set(service, account, expected.copyOf())
            val before = file().readBytes()
            val failing = KeystoreCredentials(context) { directoryCalls++ }
            assertThrows(AndroidException.Credentials::class.java) {
                failing.set(service, account, rejected)
            }
            assertEquals(0, directoryCalls)
            assertTrue(rejected.all { it == 0.toByte() })
            assertArrayEquals(before, file().readBytes())
            assertArrayEquals(expected, KeystoreCredentials(context).get(service, account))
        } finally { file().delete(); expected.fill(0); rejected.fill(0) }
    }

    @Test fun malformedVersionAndWrongAuthenticatedAddressNeverBecomeMissing() {
        val expected = "PUBLIC authenticated address fixture".toByteArray()
        val other = file("PUBLIC different authenticated account")
        try {
            val credentials = KeystoreCredentials(context)
            assertNull(credentials.get(service, account))
            credentials.set(service, account, expected.copyOf())
            val ciphertext = file().readBytes()
            other.writeBytes(ciphertext)
            assertThrows(AndroidException.ProfileInvalid::class.java) {
                credentials.get(service, "PUBLIC different authenticated account")
            }
            ciphertext[0] = 99
            file().writeBytes(ciphertext)
            assertThrows(AndroidException.ProfileInvalid::class.java) { credentials.get(service, account) }
            file().writeBytes(byteArrayOf(1, 2))
            assertThrows(AndroidException.ProfileInvalid::class.java) { credentials.get(service, account) }
        } finally { file().delete(); other.delete(); expected.fill(0) }
    }
}
