package dev.taypeer

import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import dev.taypeer.bridge.FormKind
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/** Actual settings/dirty navigation gestures use the production state and real isolated worker. */
class PolicyUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val state get() = document.state
    @Before fun create() { document = PublicDocumentUiFixture(compose); document.create() }
    @After fun restore() { if (::document.isInitialized) document.restorePreferences() }
    private fun tap(tag: String) {
        compose.runOnIdle { compose.activity.getSystemService(InputMethodManager::class.java)
            .hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0) }
        compose.waitUntil(10_000) { compose.activity.window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.ime()) == false }
        val node = compose.onNodeWithTag(tag)
        if (generateSequence(node.fetchSemanticsNode().parent) { it.parent }.any { SemanticsActions.ScrollBy in it.config }) node.performScrollTo()
        compose.onNodeWithTag(tag).assertIsDisplayed().assertIsEnabled().performTouchInput { click() }
    }
    private fun openSettings() {
        document.menu("open-settings"); document.waitFor("settings-scope")
        tap("settings-scope"); tap("settings-scope-database")
        document.waitFor("settings-protection"); document.waitFor("settings-limits")
    }

    @Test fun separateSettingsRoutesShowActualMeasurementsAndExplicitCalibrationDoesNotSave() {
        val group = document.group("PUBLIC policy UI group")
        val entry = document.entry(group, PublicEntryFields(title = "PUBLIC policy UI entry"))
        val before = document.savedBytes()
        compose.runOnIdle { state.showHistory(FormKind.ENTRY, entry) }
        document.await { state.history?.target == entry && !state.navigating }
        var history = emptyList<String>()
        compose.runOnIdle { history = state.history!!.rows.map { it.id }; state.dismissHistory() }
        document.showGroup(group)
        openSettings(); tap("settings-protection")
        document.await { state.policySettings.page == PolicyPage.PROTECTION && state.policySettings.details != null && !state.policySettings.busy }
        val details = state.policySettings.details!!
        assertTrue(details.passwordDerivationMs!! > 0uL)
        compose.onNodeWithTag("policy-memory").assertIsNotEnabled()
        compose.onNodeWithTag("policy-password").assertDoesNotExist()
        compose.onNodeWithTag("policy-actual-time").assertExists()
        tap("calibrate-policy")
        document.await { !state.policySettings.calibrating && state.policySettings.preview != null }
        compose.onNodeWithTag("policy-calibration-preview").performScrollTo().assertIsDisplayed()
        assertEquals(details.policy.kdfTargetMs, state.policySettings.preview!!.targetMs)
        assertEquals(details.passwordDerivationMs, state.policySettings.details!!.passwordDerivationMs)
        assertArrayEquals(before, document.savedBytes())
        tap("cancel-form")
        document.await { state.policySettings.page == null }
        openSettings(); tap("settings-limits")
        document.await { state.policySettings.page == PolicyPage.LIMITS && state.policySettings.details != null && !state.policySettings.busy }
        compose.onNodeWithTag("policy-attachment").assertTextEquals(PolicyUnits.editableMib(details.policy.attachmentBytes))
        compose.onNodeWithTag("policy-total").assertTextEquals(PolicyUnits.editableMib(details.policy.totalAttachmentBytes))
        compose.onNodeWithTag("policy-used").assertExists()
        compose.onNodeWithTag("policy-password").assertDoesNotExist()
        tap("cancel-form")
        compose.runOnIdle { state.showHistory(FormKind.ENTRY, entry) }
        document.await { state.history?.target == entry && !state.navigating }
        assertEquals(history, state.history!!.rows.map { it.id })
        assertArrayEquals(before, document.savedBytes())
    }

    @Test fun openingProtectionFromDirtyDatabaseMetadataKeepsStayDiscardAndOneExplicitSave() {
        val before = document.savedBytes()
        val notes = "PUBLIC policy navigation exact description"
        fun editMetadata() {
            compose.runOnIdle { state.beginMetadata(FormKind.DATABASE) }
            document.await { state.metadata != null && !state.navigating }
            compose.onNodeWithTag("metadata-description").performTextReplacement(notes)
            document.await { state.activeForm?.dirty == true && !state.editingBlocked }
        }
        editMetadata()
        var draft = ""
        compose.runOnIdle { draft = state.activeForm!!.draft }
        tap("metadata-protection"); document.waitFor("unsaved-changes")
        tap("navigation-stay"); document.waitAbsent("unsaved-changes")
        compose.runOnIdle { assertEquals(draft, state.activeForm!!.draft); assertNull(state.policySettings.page); assertEquals(notes, state.metadata!!.description) }
        assertArrayEquals(before, document.savedBytes())
        tap("metadata-protection"); document.waitFor("unsaved-changes"); tap("navigation-discard")
        document.await { state.policySettings.details != null && !state.policySettings.busy }
        assertArrayEquals(before, document.savedBytes())
        tap("cancel-form")
        editMetadata()
        tap("metadata-protection"); document.waitFor("unsaved-changes"); tap("navigation-save")
        document.await { state.policySettings.details != null && !state.policySettings.busy && state.activeForm == null }
        assertEquals(notes, state.overview!!.description)
        val saved = document.savedBytes()
        assertFalse(before.contentEquals(saved))
        tap("cancel-form")
        compose.runOnIdle { state.showHistory(FormKind.DATABASE, null) }
        document.await { state.history?.kind == FormKind.DATABASE && !state.navigating }
        assertEquals(2, state.history!!.rows.size)
        assertArrayEquals(saved, document.savedBytes())
    }
}
