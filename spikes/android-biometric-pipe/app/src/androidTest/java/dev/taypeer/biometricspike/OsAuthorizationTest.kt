package dev.taypeer.biometricspike

import android.os.ParcelFileDescriptor
import android.view.accessibility.AccessibilityNodeInfo
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.BiometricKeyPipe
import dev.taypeer.platform.BiometricOsKey
import java.io.File
import java.security.GeneralSecurityException
import java.security.KeyFactory
import java.security.spec.MGF1ParameterSpec
import java.security.spec.X509EncodedKeySpec
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Real Android Keystore and OS CryptoObject. The driver supplies a PUBLIC emulator finger. */
@RunWith(AndroidJUnit4::class)
class OsAuthorizationTest {
    private class PromptFailure(val code: Int) : Exception("PUBLIC OS prompt error code=$code")
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val scope = "50".repeat(32)
    private val oaepParameters = OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT)
    private val marker = File(context.filesDir, "PUBLIC-biometric-prompt-ready")

    private fun promptVisible(title: String): Boolean {
        val root = InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow ?: return false
        val pending = java.util.ArrayDeque<AccessibilityNodeInfo>().apply { add(root) }
        var found = false
        var examined = 0
        while (pending.isNotEmpty()) {
            val node = pending.removeFirst()
            if (examined++ < 2048) {
                found = found || node.text?.toString() == title
                repeat(minOf(node.childCount, 100)) { index -> node.getChild(index)?.let(pending::addLast) }
            }
            @Suppress("DEPRECATION")
            node.recycle()
        }
        return found
    }

    private fun awaitPrompt(title: String, visible: Boolean) {
        val deadline = android.os.SystemClock.elapsedRealtime() + 5_000
        while (promptVisible(title) != visible && android.os.SystemClock.elapsedRealtime() < deadline) Thread.sleep(50)
        assertEquals("The exact OS prompt must reach the observed visibility state", visible, promptVisible(title))
    }

    private fun wrapped(key: BiometricOsKey): ByteArray = Cipher.getInstance("RSA/ECB/OAEPWithSHA-256AndMGF1Padding").apply {
        init(Cipher.ENCRYPT_MODE, KeyFactory.getInstance("RSA").generatePublic(X509EncodedKeySpec(key.publicSpki())), oaepParameters)
        val actual = this.parameters.getParameterSpec(OAEPParameterSpec::class.java)
        assertEquals("SHA-256", actual.digestAlgorithm)
        assertEquals("SHA-1", (actual.mgfParameters as MGF1ParameterSpec).digestAlgorithm)
    }.doFinal(ByteArray(32) { 0x50 })

    private fun input(pipe: ParcelFileDescriptor): ByteArray = ParcelFileDescriptor.AutoCloseInputStream(pipe).use {
        val bytes = ByteArray(33)
        var count = 0
        while (count < bytes.size) {
            val read = it.read(bytes, count, bytes.size - count)
            if (read < 0) break
            count += read
        }
        assertTrue("Key output must have exactly bounded EOF", count <= 32)
        bytes.copyOf(count)
    }

    private fun assertUnauthenticated(key: BiometricOsKey, ciphertext: ByteArray) {
        val descriptors = ParcelFileDescriptor.createPipe()
        BiometricKeyPipe.open(descriptors[1]).use { pipe ->
            assertThrows(GeneralSecurityException::class.java) { pipe.finish(key.decryptCipher(), ciphertext) }
            assertTrue(input(descriptors[0]).isEmpty())
        }
    }

    private fun prompt(scenario: ActivityScenario<PipeSpikeActivity>, cipher: Cipher, title: String,
        rejected: CompletableFuture<Unit>? = null): Pair<BiometricPrompt, CompletableFuture<BiometricPrompt.AuthenticationResult>> {
        val completed = CompletableFuture<BiometricPrompt.AuthenticationResult>()
        lateinit var prompt: BiometricPrompt
        scenario.onActivity { activity ->
            prompt = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) { completed.complete(result) }
                override fun onAuthenticationFailed() { rejected?.complete(Unit) }
                override fun onAuthenticationError(code: Int, message: CharSequence) {
                    // OS messages are intentionally excluded from assertions and evidence.
                    completed.completeExceptionally(PromptFailure(code))
                }
            })
            prompt.authenticate(BiometricPrompt.PromptInfo.Builder()
                .setTitle(title)
                .setNegativeButtonText("PUBLIC master fallback")
                .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                .build(), BiometricPrompt.CryptoObject(cipher))
        }
        return prompt to completed
    }

    @Test(timeout = 20_000) fun actualStrongKeyCannotDecryptWithoutOsAuthorizationAndRejectsChangedRegistryBinding() {
        val key = BiometricOsKey.create(context, scope)
        try {
            val ciphertext = wrapped(key)
            assertUnauthenticated(key, ciphertext)
            assertThrows(IllegalStateException::class.java) {
                BiometricOsKey.registered(key.registryAlias(), scope, key.publicSpki().copyOf().apply { this[lastIndex] = (this[lastIndex].toInt() xor 1).toByte() })
            }
            assertThrows(IllegalArgumentException::class.java) {
                BiometricOsKey.registered(key.registryAlias(), "51".repeat(32), key.publicSpki())
            }
            val registered = BiometricOsKey.registered(key.registryAlias(), scope, key.publicSpki())
            assertArrayEquals(key.publicSpki(), registered.publicSpki())
        } finally { key.delete() }
    }

    @Test(timeout = 40_000) fun actualOsCryptoObjectDecryptsThroughProductJniAndFreshCipherRequiresAnotherPrompt() {
        val key = BiometricOsKey.create(context, scope)
        try {
            val ciphertext = wrapped(key)
            ActivityScenario.launch(PipeSpikeActivity::class.java).use { scenario ->
                val cipher = key.decryptCipher()
                val title = "PUBLIC JCA Keystore authorization"
                val (prompt, completed) = prompt(scenario, cipher, title)
                try {
                    awaitPrompt(title, true)
                    marker.writeText("touch-1\n")
                    val result = completed.get(20, TimeUnit.SECONDS)
                    assertSame("The exact initialized Cipher must return from the real OS prompt", cipher, result.cryptoObject?.cipher)
                    assertEquals(BiometricPrompt.AUTHENTICATION_RESULT_TYPE_BIOMETRIC, result.authenticationType)
                    val descriptors = ParcelFileDescriptor.createPipe()
                    try {
                        BiometricKeyPipe.open(descriptors[1]).use { pipe ->
                            assertTrue(pipe.finish(result.cryptoObject!!.cipher!!, ciphertext))
                            PublicIsolatedNativeProbe.verify(descriptors[0], expectedExact = true)
                            assertFalse(pipe.finish(cipher, ciphertext))
                        }
                    } finally { descriptors[0].close(); descriptors[1].close() }
                    assertUnauthenticated(key, ciphertext)
                } finally {
                    marker.delete()
                    scenario.onActivity { prompt.cancelAuthentication() }
                }
            }
        } finally { key.delete() }
    }

    @Test(timeout = 20_000) fun cancelledRealOsPromptDoesNotAuthorizeTheInitializedCipher() {
        val key = BiometricOsKey.create(context, scope)
        try {
            val ciphertext = wrapped(key)
            ActivityScenario.launch(PipeSpikeActivity::class.java).use { scenario ->
                val cipher = key.decryptCipher()
                val title = "PUBLIC cancellation Keystore authorization"
                val (prompt, completed) = prompt(scenario, cipher, title)
                awaitPrompt(title, true)
                var attached = false
                scenario.onActivity { activity ->
                    attached = activity.supportFragmentManager.findFragmentByTag("androidx.biometric.BiometricFragment") != null
                    prompt.cancelAuthentication()
                }
                val cancellation = try {
                    assertThrows(java.util.concurrent.ExecutionException::class.java) { completed.get(5, TimeUnit.SECONDS) }
                } catch (failure: AssertionError) {
                    throw AssertionError("PUBLIC cancel diagnostic: ownedFragmentAttached=$attached, visible=${promptVisible(title)}", failure)
                }
                val cause = cancellation.cause
                assertTrue("The exact prompt must report OS cancellation", cause is PromptFailure &&
                    cause.code in setOf(BiometricPrompt.ERROR_CANCELED, BiometricPrompt.ERROR_USER_CANCELED))
                awaitPrompt(title, false)
                assertFalse("Cancellation must never deliver a successful result", completed.isDone && !completed.isCompletedExceptionally)
                val descriptors = ParcelFileDescriptor.createPipe()
                BiometricKeyPipe.open(descriptors[1]).use { pipe ->
                    assertThrows(GeneralSecurityException::class.java) { pipe.finish(cipher, ciphertext) }
                    assertTrue(input(descriptors[0]).isEmpty())
                }
            }
        } finally { key.delete() }
    }

    @Test(timeout = 40_000) fun rustOaepWrapperDecryptsOnlyThroughTheExactOsCryptoObjectAndPrivatePipe() {
        val key = BiometricOsKey.create(context, scope)
        val publicSpki = File(context.filesDir, "PUBLIC-rust-spki.b64")
        val ciphertextFile = File(context.filesDir, "PUBLIC-rust-wrapped.bin")
        try {
            ciphertextFile.delete()
            publicSpki.writeText(android.util.Base64.encodeToString(key.publicSpki(), android.util.Base64.NO_WRAP))
            marker.writeText("wrap-rust\n")
            val deadline = android.os.SystemClock.elapsedRealtime() + 15_000
            while (!ciphertextFile.exists() && android.os.SystemClock.elapsedRealtime() < deadline) Thread.sleep(50)
            assertTrue("The PUBLIC Rust OAEP fixture must be delivered by the external driver", ciphertextFile.exists())
            val ciphertext = ciphertextFile.readBytes()
            assertEquals(256, ciphertext.size)
            marker.delete()
            ActivityScenario.launch(PipeSpikeActivity::class.java).use { scenario ->
                val cipher = key.decryptCipher()
                val title = "PUBLIC Rust Keystore authorization"
                val (prompt, completed) = prompt(scenario, cipher, title)
                try {
                    awaitPrompt(title, true)
                    marker.writeText("touch-1\n")
                    val result = completed.get(20, TimeUnit.SECONDS)
                    assertSame(cipher, result.cryptoObject?.cipher)
                    val descriptors = ParcelFileDescriptor.createPipe()
                    try {
                        BiometricKeyPipe.open(descriptors[1]).use { pipe ->
                            assertTrue(pipe.finish(result.cryptoObject!!.cipher!!, ciphertext))
                            PublicIsolatedNativeProbe.verify(descriptors[0], expectedExact = true)
                        }
                    } finally { descriptors[0].close(); descriptors[1].close() }
                    assertUnauthenticated(key, ciphertext)
                } finally {
                    marker.delete()
                    scenario.onActivity { prompt.cancelAuthentication() }
                }
            }
        } finally {
            marker.delete(); publicSpki.delete(); ciphertextFile.delete()
            key.delete()
        }
    }

    @Test(timeout = 20_000) fun directPublicAndroidPromptCancellationClosesTheExactOsRequest() {
        val key = BiometricOsKey.create(context, scope)
        try {
            val ciphertext = wrapped(key)
            ActivityScenario.launch(PipeSpikeActivity::class.java).use { scenario ->
                val title = "PUBLIC platform cancellation authorization"
                val cipher = key.decryptCipher()
                val signal = android.os.CancellationSignal()
                val completed = CompletableFuture<Int>()
                scenario.onActivity { activity ->
                    android.hardware.biometrics.BiometricPrompt.Builder(activity)
                        .setTitle(title)
                        .setAllowedAuthenticators(android.hardware.biometrics.BiometricManager.Authenticators.BIOMETRIC_STRONG)
                        .setNegativeButton("PUBLIC master fallback", ContextCompat.getMainExecutor(activity)) { _, _ -> signal.cancel() }
                        .build().authenticate(android.hardware.biometrics.BiometricPrompt.CryptoObject(cipher), signal,
                            ContextCompat.getMainExecutor(activity), object : android.hardware.biometrics.BiometricPrompt.AuthenticationCallback() {
                                override fun onAuthenticationError(code: Int, message: CharSequence) { completed.complete(code) }
                                override fun onAuthenticationSucceeded(result: android.hardware.biometrics.BiometricPrompt.AuthenticationResult) {
                                    completed.completeExceptionally(AssertionError("PUBLIC cancelled prompt authenticated"))
                                }
                            })
                }
                awaitPrompt(title, true)
                scenario.onActivity { signal.cancel() }
                assertTrue(completed.get(5, TimeUnit.SECONDS) in setOf(BiometricPrompt.ERROR_CANCELED, BiometricPrompt.ERROR_USER_CANCELED))
                awaitPrompt(title, false)
                val descriptors = ParcelFileDescriptor.createPipe()
                BiometricKeyPipe.open(descriptors[1]).use { pipe ->
                    assertThrows(GeneralSecurityException::class.java) { pipe.finish(cipher, ciphertext) }
                    assertTrue(input(descriptors[0]).isEmpty())
                }
            }
        } finally { key.delete() }
    }

    @Test(timeout = 20_000) fun actualStartedSensorReportsCancellationAfterOneRejectedPublicFinger() {
        val key = BiometricOsKey.create(context, scope)
        try {
            val ciphertext = wrapped(key)
            ActivityScenario.launch(PipeSpikeActivity::class.java).use { scenario ->
                val cipher = key.decryptCipher()
                val title = "PUBLIC started sensor cancellation"
                val rejected = CompletableFuture<Unit>()
                val (prompt, completed) = prompt(scenario, cipher, title, rejected)
                try {
                    awaitPrompt(title, true)
                    marker.writeText("touch-2\n")
                    rejected.get(5, TimeUnit.SECONDS)
                    marker.delete()
                    scenario.onActivity { prompt.cancelAuthentication() }
                    val cancellation = assertThrows(java.util.concurrent.ExecutionException::class.java) { completed.get(5, TimeUnit.SECONDS) }
                    val cause = cancellation.cause
                    assertTrue(cause is PromptFailure && cause.code == BiometricPrompt.ERROR_CANCELED)
                    awaitPrompt(title, false)
                    assertUnauthenticated(key, ciphertext)
                } finally { marker.delete(); scenario.onActivity { prompt.cancelAuthentication() } }
            }
        } finally { key.delete() }
    }
}
