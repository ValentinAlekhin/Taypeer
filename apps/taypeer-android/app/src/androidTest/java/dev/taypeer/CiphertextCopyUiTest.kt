package dev.taypeer

import android.accessibilityservice.AccessibilityService
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/** Real product action/ActivityResult registration retains an opaque capture across OS lifecycle. */
class CiphertextCopyUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture

    @Before fun createPublicDatabase() {
        document = PublicDocumentUiFixture(compose)
        document.create()
    }
    @After fun cleanupPublicPresentation() {
        if (::document.isInitialized) {
            compose.runOnIdle { document.state.databaseCopy.cancel(); document.state.lock() }
            document.restorePreferences()
        }
    }
    private fun prepare() {
        document.menu("save-database-copy")
        document.waitFor("ciphertext-copy-screen")
        document.await { document.state.databaseCopy.phase == CiphertextCopyPhase.READY }
        compose.onNodeWithTag("choose-copy-location").assertIsEnabled()
    }

    @Test fun readonlyFilenameAndFrozenCaptureSurviveRecreationWithoutAnOutputOrSecret() {
        val source = document.savedBytes()
        var filename = ""
        compose.runOnIdle { filename = document.state.overview!!.name + ".taypeer" }
        prepare()
        compose.onNodeWithTag("database-copy-filename").assertExists()
        compose.onNodeWithText(filename).assertExists()
        var operation = ""
        var fingerprint = ""
        compose.runOnIdle {
            operation = document.state.databaseCopy.operation!!
            fingerprint = document.state.databaseCopy.identity!!.fingerprint
            assertEquals(filename, document.state.databaseCopy.filename)
        }
        compose.activityRule.scenario.recreate()
        document.waitFor("ciphertext-copy-screen")
        compose.runOnIdle {
            assertEquals(operation, document.state.databaseCopy.operation)
            assertEquals(fingerprint, document.state.databaseCopy.identity!!.fingerprint)
            assertEquals(CiphertextCopyPhase.READY, document.state.databaseCopy.phase)
            assertNull(document.state.databaseCopy.pendingPicker)
            assertTrue(document.state.revealed.isEmpty())
        }
        assertArrayEquals(source, document.savedBytes())
        compose.onNodeWithTag("cancel-database-copy").performClick()
        document.waitAbsent("ciphertext-copy-screen")
        compose.runOnIdle { assertNull(document.state.databaseCopy.identity); assertNull(document.state.databaseCopy.operation) }
    }

    @Test fun actualSystemPickerBackgroundLocksTheDocumentAndBackCancelsWithoutCopySuccess() {
        val source = document.savedBytes()
        prepare()
        compose.onNodeWithTag("choose-copy-location").performClick()
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        try {
            compose.waitUntil(20_000) {
                automation.rootInActiveWindow?.packageName?.toString()?.endsWith(".documentsui") == true &&
                    !compose.activity.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
            }
            // This is the genuine external picker/onStop path, not a simulated null URI.
            assertNull(document.state.overview)
            assertTrue(document.state.revealed.isEmpty())
            assertTrue(document.state.databaseCopy.showing)
            assertEquals(CiphertextCopyPhase.SELECTING, document.state.databaseCopy.phase)
            assertTrue(automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK))
            document.await { compose.activity.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED) && !document.state.databaseCopy.showing }
            document.waitFor("master-password")
            compose.onNodeWithTag("database-copy-saved").assertDoesNotExist()
            compose.runOnIdle {
                assertNull(document.state.overview)
                assertNull(document.state.entry)
                assertNull(document.state.inspectedEntry)
                assertNull(document.state.databaseCopy.operation)
                assertNull(document.state.databaseCopy.identity)
            }
            assertArrayEquals(source, document.savedBytes())
        } finally {
            if (automation.rootInActiveWindow?.packageName?.toString()?.endsWith(".documentsui") == true)
                automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
        }
    }

    @Test fun registeredLockedCopyCanPrepareExportWithAnEmptyMasterPassword() {
        val source = document.savedBytes()
        compose.runOnIdle { document.state.lock() }
        document.waitFor("master-password")
        compose.onNodeWithTag("master-password").assertTextEquals("")
        compose.onNodeWithTag("save-locked-database-copy").performScrollTo().assertIsEnabled().performClick()
        document.waitFor("ciphertext-copy-screen")
        document.await { document.state.databaseCopy.phase == CiphertextCopyPhase.READY }
        compose.runOnIdle { assertNull(document.state.overview); assertNotNull(document.state.databaseCopy.identity) }
        compose.onNodeWithTag("choose-copy-location").assertIsEnabled()
        assertArrayEquals(source, document.savedBytes())
    }
}
