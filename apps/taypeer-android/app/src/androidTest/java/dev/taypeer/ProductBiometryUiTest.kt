package dev.taypeer

import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.BiometryState
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Enrolled PUBLIC emulator only: ordinary UI actions, actual OS prompt, real isolated worker. */
class ProductBiometryUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val fixture = PublicDocumentUiFixture(compose)
    private val state get() = fixture.state
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val app get() = compose.activity.application as TaypeerApplication
    private val marker get() = File(app.filesDir, "PUBLIC-biometric-prompt-ready")

    private fun awaitOsPrompt() {
        val title = fixture.text(R.string.biometric_prompt_title)
        val deadline = SystemClock.elapsedRealtime() + 5_000
        while (!osNode(title) { true } && SystemClock.elapsedRealtime() < deadline) Thread.sleep(50)
        assertTrue("Actual product OS prompt must appear", osNode(title) { true })
    }
    private fun osNode(text: String, action: (AccessibilityNodeInfo) -> Boolean): Boolean {
        val root = instrumentation.uiAutomation.rootInActiveWindow ?: return false
        val nodes = java.util.ArrayDeque<AccessibilityNodeInfo>().apply { add(root) }
        var found = false
        var count = 0
        while (nodes.isNotEmpty()) {
            val node = nodes.removeFirst()
            if (count++ < 2048) {
                if (!found && node.text?.toString() == text) found = action(node)
                repeat(minOf(node.childCount, 100)) { node.getChild(it)?.let(nodes::addLast) }
            }
            @Suppress("DEPRECATION") node.recycle()
        }
        return found
    }
    private fun settings() {
        fixture.menu("open-settings")
        fixture.waitFor("biometric-settings")
        fixture.await { state.biometry.status != null && !state.biometry.busy }
    }
    private fun startEnrollment() {
        settings()
        compose.onNodeWithTag("biometric-enable").performScrollTo().assertIsEnabled().performClick()
        awaitOsPrompt()
    }
    private fun authenticate() {
        marker.writeText("touch-1\n")
        try { fixture.await { !state.biometry.busy && !state.opening } }
        finally { marker.delete() }
    }
    private inline fun scenario(test: () -> Unit) {
        marker.delete()
        fixture.create()
        try { test() }
        finally {
            app.osBiometry.beforeProvider = null
            marker.delete()
            fixture.restorePreferences()
        }
    }

    @Test fun realEnrollmentAndKeyedOpenKeepSavedContentAndDisableRequiresExactGeneration() = scenario {
        val group = fixture.group("PUBLIC biometric group")
        val entry = fixture.entry(group, PublicEntryFields(title = "PUBLIC biometric unchanged entry"))
        val bytes = fixture.savedBytes()
        val database = state.overview!!.database
        startEnrollment(); authenticate()
        compose.runOnIdle { assertEquals(BiometryState.ENABLED, state.biometry.status?.state); assertFalse(state.biometry.failed) }
        compose.onNodeWithTag("biometric-disable").assertIsEnabled()
        assertArrayEquals(bytes, fixture.savedBytes())
        compose.onNodeWithTag("navigation-back").performClick()
        fixture.waitFor("group-menu")
        fixture.menu("lock-database")
        fixture.waitFor("master-password")
        fixture.waitFor("biometric-unlock")
        compose.onNodeWithTag("biometric-unlock").performScrollTo().performClick()
        awaitOsPrompt(); authenticate()
        fixture.await { state.overview?.database == database && !state.opening && !state.navigating }
        assertArrayEquals(bytes, fixture.savedBytes())
        fixture.inspect(group, entry)
        compose.onNode(hasText("PUBLIC biometric unchanged entry") and hasAnyAncestor(hasTestTag("route-inspected"))).assertExists()
        settings()
        compose.onNodeWithTag("biometric-disable").performScrollTo().performClick()
        fixture.await { state.biometry.status?.state == BiometryState.DISABLED && !state.biometry.busy }
        assertArrayEquals(bytes, fixture.savedBytes())
        compose.onNodeWithTag("navigation-back").performClick()
        fixture.waitFor("group-menu")
        fixture.menu("lock-database")
        fixture.waitFor("master-password")
        fixture.await { state.biometry.status?.state == BiometryState.DISABLED && !state.biometry.busy }
        compose.onNodeWithTag("biometric-unlock").assertDoesNotExist()
        compose.onNodeWithTag("master-password").performTextReplacement("PUBLIC lifecycle UI master password")
        compose.tapWorkingCopyUnlock(database)
        fixture.await { state.overview?.database == database && !state.navigating }
        assertArrayEquals(bytes, fixture.savedBytes())
    }

    @Test fun realOsNegativeButtonNeverEnablesSetupAndMasterFallbackKeepsTheFile() = scenario {
        val bytes = fixture.savedBytes()
        startEnrollment()
        val deadline = SystemClock.elapsedRealtime() + 5_000
        var clicked = false
        while (!clicked && SystemClock.elapsedRealtime() < deadline) {
            clicked = osNode(fixture.text(R.string.biometric_master_password)) {
                it.isEnabled && it.performAction(AccessibilityNodeInfo.ACTION_CLICK)
            }
            if (!clicked) Thread.sleep(50)
        }
        assertTrue("Actual OS negative button must be clicked", clicked)
        fixture.await { !state.biometry.busy && state.biometry.status?.state == BiometryState.DISABLED }
        compose.runOnIdle { assertTrue(state.biometry.cancelled); assertNotNull(state.overview) }
        assertArrayEquals(bytes, fixture.savedBytes())
        compose.onNodeWithTag("navigation-back").performClick()
        fixture.reopen()
        assertArrayEquals(bytes, fixture.savedBytes())
    }

    @Test fun backgroundAfterRealOsSuccessBeforeProviderCannotPublishOrReviveAndMasterReopens() = scenario {
        val reached = CountDownLatch(1)
        val release = CountDownLatch(1)
        val bytes = fixture.savedBytes()
        val database = state.overview!!.database
        app.osBiometry.beforeProvider = {
            reached.countDown()
            check(release.await(10, TimeUnit.SECONDS)) { "PUBLIC provider barrier not released" }
        }
        try {
            startEnrollment()
            marker.writeText("touch-1\n")
            assertTrue("Actual OS callback must reach private provider barrier", reached.await(20, TimeUnit.SECONDS))
            marker.delete()
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            compose.runOnIdle { assertNull(state.overview); assertNull(state.biometry.status); assertFalse(state.biometry.busy) }
            release.countDown()
            app.osBiometry.beforeProvider = null
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            fixture.waitFor("master-password")
            fixture.await { state.biometry.status?.state == BiometryState.DISABLED && !state.biometry.busy }
            compose.onNodeWithTag("biometric-unlock").assertDoesNotExist()
            compose.onNodeWithTag("master-password").performTextReplacement("PUBLIC lifecycle UI master password")
            compose.tapWorkingCopyUnlock(database)
            fixture.await { state.overview?.database == database && !state.navigating }
            assertArrayEquals(bytes, fixture.savedBytes())
        } finally { release.countDown(); app.osBiometry.beforeProvider = null }
    }
}
