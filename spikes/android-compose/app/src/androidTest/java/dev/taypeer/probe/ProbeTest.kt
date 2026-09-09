package dev.taypeer.probe

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.input.TextFieldValue
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.taypeer.probe.bridge.ProbeSession
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ProbeTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val sample = "SYNTHETIC-ONLY-Жук-42"

    private fun openSample() {
        ui.onNodeWithTag("sample").performTextInput(sample)
        ui.onNodeWithTag("open").performClick()
        ui.waitForIdle()
        ui.runOnIdle { assertFalse(ui.activity.probe.status.locked) }
    }

    @Test fun generatedBindingsRoundTripUnicodeAndRejectOldGeneration() {
        val session = ProbeSession()
        try {
            val before = session.openSample(sample)
            assertEquals(sample, session.reveal(before.generation).text)
            assertEquals(sample.codePointCount(0, sample.length), before.characters.toInt())
            session.lock()
            session.openSample("SYNTHETIC-SECOND")
            assertThrows(Exception::class.java) { session.reveal(before.generation) }
        } finally { session.lock(); session.destroy() }
    }

    @Test fun composeInputRevealsExactlyAndLockClearsBothSides() {
        openSample()
        ui.onNodeWithTag("reveal").performScrollTo().performClick()
        ui.waitUntil(5000) { ui.activity.probe.revealed == sample }
        ui.onNodeWithTag("revealed").assertTextEquals(sample)
        ui.onNodeWithTag("lock").performClick()
        ui.runOnIdle {
            assertTrue(ui.activity.probe.status.locked)
            assertEquals("", ui.activity.probe.input.text)
            assertEquals("", ui.activity.probe.revealed)
        }
    }

    @Test fun issuedReplyCannotReappearAfterLockAndNewSession() {
        val issued = CompletableDeferred<Unit>()
        val delivery = CompletableDeferred<Unit>()
        ui.runOnIdle {
            ui.activity.probe.delayReply = { issued.complete(Unit); delivery.await() }
        }
        openSample()
        ui.onNodeWithTag("delayed").performScrollTo().performClick()
        ui.waitUntil(5000) { issued.isCompleted }
        ui.onNodeWithTag("lock").performClick()
        ui.onNodeWithTag("sample").performScrollTo().performTextInput("SYNTHETIC-SECOND")
        ui.onNodeWithTag("open").performClick()
        delivery.complete(Unit)
        ui.waitUntil(7000) { ui.activity.probe.message == R.string.discarded }
        ui.onNodeWithTag("revealed").assertTextEquals("")
    }

    @Test fun backgroundClearsInputAndRustSession() {
        openSample()
        ui.onNodeWithTag("sample").performTextInput("SYNTHETIC-DRAFT")
        ui.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        ui.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        ui.runOnIdle {
            assertTrue(ui.activity.probe.status.locked)
            assertEquals("", ui.activity.probe.input.text)
            assertEquals("", ui.activity.probe.revealed)
        }
    }

    @Test fun themeAndLanguageChangesKeepEditingState() {
        ui.onNodeWithTag("sample").performTextInput(sample)
        ui.onNodeWithTag("theme-dark").performClick()
        ui.onNodeWithTag("language").performClick()
        ui.runOnIdle { assertEquals(sample, ui.activity.probe.input.text) }
        ui.onNodeWithTag("theme-light").performClick()
        ui.onNodeWithTag("theme-system").performClick()
        ui.runOnIdle { assertEquals(sample, ui.activity.probe.input.text) }
    }

    @Test fun oldInputConnectionCannotRestoreDraftAfterLock() {
        ui.runOnIdle {
            val controller = ui.activity.probe
            val old = controller.status.generation
            controller.edit(TextFieldValue(sample), old)
            assertEquals(sample, controller.input.text)
            controller.lockNow()
            controller.edit(TextFieldValue("SYNTHETIC-STALE"), old)
            assertEquals("", controller.input.text)
        }
    }

    @Test fun activityRecreationKeepsSessionLockedAndUiEmpty() {
        openSample()
        ui.activityRule.scenario.recreate()
        ui.runOnIdle {
            assertTrue(ui.activity.probe.status.locked)
            assertEquals("", ui.activity.probe.input.text)
            assertEquals("", ui.activity.probe.revealed)
        }
    }
}
