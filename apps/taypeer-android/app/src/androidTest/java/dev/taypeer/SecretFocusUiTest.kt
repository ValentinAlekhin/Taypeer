package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/** A window can lose focus while Activity stays RESUMED; every visible input must mask. */
class SecretFocusUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture

    @Before fun createPublicDocument() {
        document = PublicDocumentUiFixture(compose)
        document.create()
    }

    @After fun restorePreferences() {
        if (::document.isInitialized) document.restorePreferences()
    }

    private fun loseWindowFocus() {
        compose.runOnIdle {
            assertEquals(Lifecycle.State.RESUMED, compose.activity.lifecycle.currentState)
            compose.activity.onWindowFocusChanged(false)
            assertEquals(Lifecycle.State.RESUMED, compose.activity.lifecycle.currentState)
            // This is a focus event, not a document lock or Activity stop.
            assertNotNull(document.state.overview)
        }
        compose.waitForIdle()
    }

    private fun regainWindowFocus() = compose.runOnIdle { compose.activity.onWindowFocusChanged(true) }

    @Test fun authenticationMasksOnFocusLossAndRetainsTheExactTypedPassword() {
        val password = " PUBLIC focus authentication input "
        compose.runOnIdle { document.state.lock() }
        document.waitFor("master-password")
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("reveal-master-password").performScrollTo().performClick()
        compose.onNodeWithTag("master-password").assertRenderedText(password)
        compose.runOnIdle {
            assertEquals(Lifecycle.State.RESUMED, compose.activity.lifecycle.currentState)
            compose.activity.onWindowFocusChanged(false)
            assertEquals(Lifecycle.State.RESUMED, compose.activity.lifecycle.currentState)
            assertNull(document.state.overview)
        }
        compose.onNodeWithTag("master-password").assertMaskedText(password.length)
        regainWindowFocus()
        compose.onNodeWithTag("reveal-master-password").performScrollTo().performClick()
        compose.onNodeWithTag("master-password").assertRenderedText(password)
    }

    @Test fun entryEditorMasksOnFocusLossWithoutDiscardingTheUnsavedPassword() {
        val group = document.group("PUBLIC focus editor group")
        document.entry(group, PublicEntryFields())
        compose.onNodeWithTag("edit-entry").performClick()
        document.waitFor("entry-password")
        val password = " PUBLIC focus unsaved entry password "
        compose.onNodeWithTag("entry-password").performTextReplacement(password)
        document.await { document.state.entry?.password == password && document.state.activeForm?.dirty == true }
        compose.onNodeWithTag("reveal-entry-input-password").performScrollTo().performClick()
        compose.onNodeWithTag("entry-password").assertRenderedText(password)
        var draft = ""
        compose.runOnIdle { draft = document.state.activeForm!!.draft }
        loseWindowFocus()
        compose.onNodeWithTag("entry-password").assertMaskedText(password.length)
        compose.runOnIdle {
            assertEquals(password, document.state.entry!!.password)
            assertEquals(draft, document.state.activeForm!!.draft)
            assertTrue(document.state.revealed.isEmpty())
        }
        regainWindowFocus()
        compose.onNodeWithTag("reveal-entry-input-password").performScrollTo().performClick()
        compose.onNodeWithTag("entry-password").assertRenderedText(password)
    }

    @Test fun passwordChangeMasksBothFieldsOnFocusLossAndPreservesMatchingInput() {
        document.menu("open-settings")
        document.waitFor("settings-scope")
        compose.onNodeWithTag("settings-scope").performClick()
        compose.onNodeWithTag("settings-scope-database").performClick()
        compose.onNodeWithTag("settings-change-password").performScrollTo().performClick()
        document.waitFor("password-change-screen")
        val password = " PUBLIC focus matching rotation password "
        compose.onNodeWithTag("rotation-new-password").performTextReplacement(password)
        compose.onNodeWithTag("rotation-repeat-password").performTextReplacement(password)
        compose.onNodeWithTag("reveal-rotation-password").performScrollTo().performClick()
        compose.onNodeWithTag("rotation-new-password").assertRenderedText(password)
        compose.onNodeWithTag("reveal-rotation-repeat").performScrollTo().performClick()
        compose.onNodeWithTag("rotation-repeat-password").assertRenderedText(password)
        loseWindowFocus()
        compose.onNodeWithTag("rotation-new-password").assertMaskedText(password.length)
        compose.onNodeWithTag("rotation-repeat-password").assertMaskedText(password.length)
        compose.onNodeWithTag("apply-password-change").assertIsEnabled()
        regainWindowFocus()
        compose.onNodeWithTag("reveal-rotation-password").performScrollTo().performClick()
        compose.onNodeWithTag("rotation-new-password").assertRenderedText(password)
        compose.onNodeWithTag("rotation-repeat-password").assertMaskedText(password.length)
        compose.onNodeWithTag("reveal-rotation-repeat").performScrollTo().performClick()
        compose.onNodeWithTag("rotation-repeat-password").assertRenderedText(password)
    }

    @Test fun generatorMasksOnFocusLossWithoutRegeneratingTheValueOrDiscardingOptions() {
        document.menu("open-generator")
        document.waitFor("generated-password")
        document.await { !compose.activity.generatorState.running && compose.activity.generatorState.generated.isNotEmpty() }
        var generated = ""
        var length = ""
        compose.runOnIdle {
            generated = compose.activity.generatorState.generated
            length = compose.activity.generatorState.length
            assertFalse(compose.activity.generatorState.revealed)
        }
        compose.onNodeWithTag("generator-reveal").performScrollTo().performClick()
        compose.onNodeWithTag("generated-password").assertRenderedText(generated)
        loseWindowFocus()
        compose.onNodeWithTag("generated-password").assertMaskedText(generated.length)
        compose.runOnIdle {
            assertFalse(compose.activity.generatorState.revealed)
            assertEquals(generated, compose.activity.generatorState.generated)
            assertEquals(length, compose.activity.generatorState.length)
        }
        regainWindowFocus()
        compose.onNodeWithTag("generator-reveal").performScrollTo().performClick()
        compose.onNodeWithTag("generated-password").assertRenderedText(generated)
        compose.onNodeWithTag("close-generator").performClick()
        document.waitAbsent("generated-password")
        compose.runOnIdle { assertTrue(compose.activity.generatorState.generated.isEmpty()) }
    }
}
