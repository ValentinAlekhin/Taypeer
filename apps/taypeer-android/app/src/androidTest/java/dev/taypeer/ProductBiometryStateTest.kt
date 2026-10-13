package dev.taypeer

import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Enrolled PUBLIC emulator: only reply delivery is delayed; Native preparation and OS proof are real. */
class ProductBiometryStateTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val fixture = PublicDocumentUiFixture(compose)
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val app get() = compose.activity.application as TaypeerApplication
    private val marker get() = File(app.filesDir, "PUBLIC-biometric-prompt-ready")

    @Test fun lateOldPreparationCannotDisownNewAttemptOrPublishAfterItsIndependentClear() {
        val oldPrepared = CountDownLatch(1)
        val releaseOld = CountDownLatch(1)
        val providerReached = CountDownLatch(1)
        val releaseProvider = CountDownLatch(1)
        val preparations = AtomicInteger()
        val lifetime = SupervisorJob()
        val scope = CoroutineScope(lifetime + Dispatchers.Main.immediate)
        val local = BiometryUiState(scope, { session ->
            val actual = session.prepareBiometricEnrollment()
            if (preparations.incrementAndGet() == 1) {
                oldPrepared.countDown()
                try {
                    check(releaseOld.await(20, TimeUnit.SECONDS)) { "PUBLIC old preparation barrier not released" }
                } catch (failure: Exception) {
                    actual.cancel(); actual.close(); throw failure
                }
            }
            actual
        }) { app.host.await() }
        marker.delete()
        try {
            fixture.create()
            lateinit var first: DocumentSession
            lateinit var firstCopy: WorkingCopyView
            compose.runOnIdle {
                firstCopy = fixture.state.catalog.single { it.database == fixture.state.overview!!.database }
                assertEquals("PUBLIC captured", fixture.state.readPreviewDiagnostic { first = it; "PUBLIC captured" })
                fixture.state.create("PUBLIC biometric second copy", null, "PUBLIC lifecycle UI master password", newOperation())
            }
            fixture.await { fixture.state.overview?.database != firstCopy.database && fixture.state.overview != null &&
                !fixture.state.opening && !fixture.state.navigating &&
                fixture.state.catalog.any { it.database == fixture.state.overview?.database } }
            lateinit var second: DocumentSession
            lateinit var secondCopy: WorkingCopyView
            compose.runOnIdle {
                secondCopy = fixture.state.catalog.single { it.database == fixture.state.overview!!.database }
                assertEquals("PUBLIC captured", fixture.state.readPreviewDiagnostic { second = it; "PUBLIC captured" })
                assertNotEquals(firstCopy.database, secondCopy.database)
                assertEquals(SessionAccess.OPEN, first.access())
                assertEquals(SessionAccess.OPEN, second.access())
            }
            val host = runBlocking { app.host.await() }
            assertEquals(BiometryState.DISABLED, host.biometryStatus(firstCopy).state)
            assertEquals(BiometryState.DISABLED, host.biometryStatus(secondCopy).state)
            val firstBytes = File(firstCopy.path).readBytes()
            val secondBytes = File(secondCopy.path).readBytes()
            lateinit var oldJob: Job
            compose.runOnIdle {
                local.enable(firstCopy, first)
                oldJob = lifetime.children.single()
            }
            assertTrue("Old actual Native preparation must finish before its delayed delivery",
                oldPrepared.await(20, TimeUnit.SECONDS))
            app.osBiometry.beforeProvider = {
                providerReached.countDown()
                check(releaseProvider.await(10, TimeUnit.SECONDS)) { "PUBLIC provider barrier not released" }
            }
            lateinit var newJob: Job
            compose.runOnIdle {
                // No background/worker lock: the two actual OPEN copies have independent Native reservations.
                local.clear()
                assertEquals(SessionAccess.OPEN, first.access())
                assertEquals(SessionAccess.OPEN, second.access())
                assertEquals("PUBLIC started", fixture.state.readPreviewDiagnostic {
                    local.enable(secondCopy, it); "PUBLIC started"
                })
                newJob = lifetime.children.single { it !== oldJob }
            }
            awaitOsPrompt()
            releaseOld.countDown()
            fixture.await { oldJob.isCompleted }
            compose.runOnIdle {
                assertTrue("Old finally must preserve the new active request", local.busy)
                assertNull(local.status)
                assertFalse(local.failed)
                assertEquals(SessionAccess.OPEN, first.access())
                assertEquals(SessionAccess.OPEN, second.access())
            }
            marker.writeText("touch-1\n")
            assertTrue("New actual OS authentication must reach its private provider",
                providerReached.await(20, TimeUnit.SECONDS))
            marker.delete()
            compose.runOnIdle {
                // The local owner alone must revoke B. Global OS/background cancellation cannot mask the race.
                assertEquals(SessionAccess.OPEN, first.access())
                assertEquals(SessionAccess.OPEN, second.access())
                local.clear()
                assertNull(local.status)
                assertNull(local.retryOperation)
                assertFalse(local.busy)
            }
            releaseProvider.countDown()
            fixture.await { newJob.isCompleted && lifetime.children.none() }
            assertEquals(BiometryState.DISABLED, host.biometryStatus(firstCopy).state)
            assertEquals(BiometryState.DISABLED, host.biometryStatus(secondCopy).state)
            assertArrayEquals(firstBytes, File(firstCopy.path).readBytes())
            assertArrayEquals(secondBytes, File(secondCopy.path).readBytes())
            compose.runOnIdle {
                assertEquals(SessionAccess.OPEN, first.access())
                assertEquals(SessionAccess.OPEN, second.access())
                assertNull(local.status)
                assertNull(local.retryOperation)
                assertFalse(local.busy)
                assertFalse(local.failed)
                assertFalse(local.cancelled)
                assertEquals(secondCopy.database, fixture.state.overview?.database)
            }
        } finally {
            releaseOld.countDown(); releaseProvider.countDown()
            app.osBiometry.beforeProvider = null
            marker.delete()
            instrumentation.runOnMainSync { local.clear(); scope.cancel() }
            fixture.restorePreferences()
        }
    }

    private fun awaitOsPrompt() {
        val title = fixture.text(R.string.biometric_prompt_title)
        val deadline = SystemClock.elapsedRealtime() + 5_000
        while (!osPrompt(title) && SystemClock.elapsedRealtime() < deadline) Thread.sleep(50)
        assertTrue("Actual product OS prompt must appear", osPrompt(title))
    }

    private fun osPrompt(title: String): Boolean {
        val root = instrumentation.uiAutomation.rootInActiveWindow ?: return false
        val nodes = java.util.ArrayDeque<AccessibilityNodeInfo>().apply { add(root) }
        var found = false
        var count = 0
        while (nodes.isNotEmpty()) {
            val node = nodes.removeFirst()
            if (count++ < 2048) {
                if (node.text?.toString() == title) found = true
                repeat(minOf(node.childCount, 100)) { node.getChild(it)?.let(nodes::addLast) }
            }
            @Suppress("DEPRECATION") node.recycle()
        }
        return found
    }
}
