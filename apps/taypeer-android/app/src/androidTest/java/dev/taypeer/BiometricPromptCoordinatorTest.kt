package dev.taypeer

import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.biometric.BiometricPrompt
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.BiometricCancellation
import dev.taypeer.platform.BiometricKeyPipe
import dev.taypeer.platform.BiometricOsKey
import dev.taypeer.platform.BiometricPromptCoordinator
import dev.taypeer.platform.BiometricPromptRequest
import java.io.File
import java.security.GeneralSecurityException
import java.security.KeyFactory
import java.security.spec.MGF1ParameterSpec
import java.security.spec.X509EncodedKeySpec
import java.util.concurrent.CompletableFuture
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import javax.crypto.Cipher
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Dedicated enrolled-emulator stage; the external driver supplies only PUBLIC finger 1. */
@RunWith(AndroidJUnit4::class)
class BiometricPromptCoordinatorTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val marker = File(context.filesDir, "PUBLIC-biometric-prompt-ready")

    private class Request(
        override val title: String,
        cipher: Cipher,
        private val pipe: BiometricKeyPipe,
        private val wrapped: ByteArray,
    ) : BiometricPromptRequest, AutoCloseable {
        override val subtitle = "PUBLIC coordinator fixture"
        override val cryptoObject = BiometricPrompt.CryptoObject(cipher)
        val authenticatedCount = AtomicInteger()
        val retireCount = AtomicInteger()
        val dismissedCount = AtomicInteger()
        val retired = CompletableFuture<BiometricCancellation>()
        val dismissed = CompletableFuture<Unit>()
        val transferred = CompletableFuture<Boolean>()
        val osError = CompletableFuture<Int>()
        private val executor = Executors.newSingleThreadExecutor()

        override fun authenticated(result: BiometricPrompt.AuthenticationResult) {
            authenticatedCount.incrementAndGet()
            assertSame(cryptoObject.cipher, result.cryptoObject?.cipher)
            assertEquals(BiometricPrompt.AUTHENTICATION_RESULT_TYPE_BIOMETRIC, result.authenticationType)
            executor.execute {
                try { transferred.complete(pipe.finish(checkNotNull(result.cryptoObject?.cipher), wrapped)) }
                catch (failure: Exception) { transferred.completeExceptionally(failure) }
            }
        }

        override fun retire(reason: BiometricCancellation) {
            retireCount.incrementAndGet()
            pipe.close()
            retired.complete(reason)
        }

        override fun promptDismissed() {
            dismissedCount.incrementAndGet()
            dismissed.complete(Unit)
        }

        override fun close() {
            pipe.close()
            executor.shutdownNow()
            check(executor.awaitTermination(5, TimeUnit.SECONDS)) { "PUBLIC fixture executor did not close" }
        }
    }

    private class Attachment(private val coordinator: BiometricPromptCoordinator) : Application.ActivityLifecycleCallbacks {
        val recreations = AtomicInteger()
        override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {
            if (activity is MainActivity) coordinator.attach(activity)
        }
        override fun onActivityStopped(activity: Activity) {
            if (activity is MainActivity && !activity.isChangingConfigurations)
                coordinator.cancel(BiometricCancellation.BACKGROUND)
        }
        override fun onActivityDestroyed(activity: Activity) {
            if (activity is MainActivity) {
                if (activity.isChangingConfigurations) recreations.incrementAndGet()
                coordinator.detach(activity, activity.isChangingConfigurations)
            }
        }
        override fun onActivityStarted(activity: Activity) = Unit
        override fun onActivityResumed(activity: Activity) = Unit
        override fun onActivityPaused(activity: Activity) = Unit
        override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
    }

    private fun wrapped(key: BiometricOsKey): ByteArray = Cipher.getInstance("RSA/ECB/OAEPWithSHA-256AndMGF1Padding").run {
        init(Cipher.ENCRYPT_MODE, KeyFactory.getInstance("RSA").generatePublic(X509EncodedKeySpec(key.publicSpki())),
            OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT))
        doFinal(ByteArray(32) { 0x50 })
    }

    private fun readBounded(input: ParcelFileDescriptor): ByteArray = ParcelFileDescriptor.AutoCloseInputStream(input).use {
        val bytes = ByteArray(33)
        var count = 0
        while (count < bytes.size) {
            val read = it.read(bytes, count, bytes.size - count)
            if (read < 0) break
            count += read
        }
        assertTrue("PUBLIC private pipe must reach bounded EOF", count <= 32)
        bytes.copyOf(count)
    }

    private fun promptVisible(title: String): Boolean {
        val root = instrumentation.uiAutomation.rootInActiveWindow ?: return false
        val pending = java.util.ArrayDeque<AccessibilityNodeInfo>().apply { add(root) }
        var found = false
        var examined = 0
        while (pending.isNotEmpty()) {
            val node = pending.removeFirst()
            if (examined++ < 2048) {
                found = found || node.text?.toString() == title
                repeat(minOf(node.childCount, 100)) { node.getChild(it)?.let(pending::addLast) }
            }
            @Suppress("DEPRECATION")
            node.recycle()
        }
        return found
    }

    private fun awaitPrompt(title: String, visible: Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 5_000
        while (promptVisible(title) != visible && SystemClock.elapsedRealtime() < deadline) Thread.sleep(50)
        assertEquals("The exact real OS prompt must reach its observed state", visible, promptVisible(title))
    }

    private fun withRequest(title: String, test: (Request, BiometricPromptCoordinator, Attachment,
        ActivityScenario<MainActivity>, ParcelFileDescriptor, BiometricOsKey) -> Unit) {
        marker.delete()
        val key = BiometricOsKey.create(context, "50".repeat(32))
        try {
            val descriptors = ParcelFileDescriptor.createPipe()
            try {
                val ciphertext = wrapped(key)
                val cipher = key.decryptCipher()
                Request(title, cipher, BiometricKeyPipe.open(descriptors[1]), ciphertext).use { request ->
                    val coordinator = BiometricPromptCoordinator({ "PUBLIC master password" }) { exact, code ->
                        assertSame(request, exact)
                        assertEquals("Retire must precede the UI error callback", 1, request.retireCount.get())
                        request.osError.complete(code)
                    }
                    val app = context.applicationContext as Application
                    val attachment = Attachment(coordinator)
                    app.registerActivityLifecycleCallbacks(attachment)
                    try {
                        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                            try { test(request, coordinator, attachment, scenario, descriptors[0], key) }
                            finally { scenario.onActivity { coordinator.cancel(BiometricCancellation.CANCEL) } }
                        }
                    } finally { app.unregisterActivityLifecycleCallbacks(attachment) }
                }
            } finally {
                descriptors.forEach { it.close() }
            }
        } finally {
            marker.delete()
            key.delete()
        }
    }

    @Test(timeout = 30_000)
    fun cancelRetiresPrivatePipeBeforeRealOsDismissalAndBlocksReplacementPrompt() {
        withRequest("PUBLIC coordinator cancellation") { request, coordinator, _, scenario, input, _ ->
            scenario.onActivity { assertTrue(coordinator.show(request)) }
            awaitPrompt(request.title, true)
            scenario.onActivity {
                coordinator.cancel(BiometricCancellation.MASTER_PASSWORD)
                assertEquals(BiometricCancellation.MASTER_PASSWORD, request.retired.getNow(null))
                assertEquals(1, request.retireCount.get())
                assertTrue("No second prompt while actual OS dismissal is pending", coordinator.awaitingDismissal)
                assertFalse(coordinator.show(request))
            }
            // The native writer closes immediately, without waiting for an OS callback.
            assertArrayEquals(ByteArray(0), readBounded(input))
            val error = request.osError.get(5, TimeUnit.SECONDS)
            assertTrue(error in setOf(BiometricPrompt.ERROR_CANCELED, BiometricPrompt.ERROR_USER_CANCELED))
            request.dismissed.get(5, TimeUnit.SECONDS)
            awaitPrompt(request.title, false)
            assertEquals(0, request.authenticatedCount.get())
            assertEquals(1, request.dismissedCount.get())
            scenario.onActivity { assertFalse(coordinator.isActive); assertFalse(coordinator.awaitingDismissal) }
        }
    }

    @Test(timeout = 45_000)
    fun recreationRebindsSameRequestAndRealStrongCryptoObjectTransfersOnlyOnce() {
        withRequest("PUBLIC coordinator recreation") { request, coordinator, attachment, scenario, input, key ->
            scenario.onActivity { assertTrue(coordinator.show(request)) }
            awaitPrompt(request.title, true)
            scenario.recreate()
            scenario.onActivity {
                assertTrue(coordinator.isActive)
                assertFalse("Recreation must never call authenticate a second time", coordinator.show(request))
            }
            assertEquals(1, attachment.recreations.get())
            awaitPrompt(request.title, true)
            marker.writeText("touch-1\n")
            assertTrue(request.transferred.get(20, TimeUnit.SECONDS))
            marker.delete()
            assertArrayEquals(ByteArray(32) { 0x50 }, readBounded(input))
            request.dismissed.get(5, TimeUnit.SECONDS)
            assertEquals(1, request.authenticatedCount.get())
            assertEquals(1, request.dismissedCount.get())
            assertEquals(0, request.retireCount.get())
            scenario.onActivity { assertFalse(coordinator.isActive) }
            val fresh = ParcelFileDescriptor.createPipe()
            BiometricKeyPipe.open(fresh[1]).use { pipe ->
                assertThrows(GeneralSecurityException::class.java) { pipe.finish(key.decryptCipher(), wrapped(key)) }
                assertArrayEquals(ByteArray(0), readBounded(fresh[0]))
            }
        }
    }
}
