package dev.taypeer

import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.HistoryRow
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*

/** Ordinary Android routes and touches backed by the real current worker and encrypted file. */
class MetadataHistoryUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val state get() = document.state

    @Before fun createPublicDocument() { document = PublicDocumentUiFixture(compose); document.create() }
    @After fun restorePreferences() { if (::document.isInitialized) document.restorePreferences() }

    private fun history(kind: FormKind, target: String?): List<HistoryRow> {
        compose.runOnIdle { state.showHistory(kind, target) }
        document.await { state.history?.kind == kind && state.history?.target == target && !state.navigating }
        document.waitFor("history-list-screen")
        var rows = emptyList<HistoryRow>()
        compose.runOnIdle { rows = state.history!!.rows }
        return rows
    }
    private fun view(revision: String) {
        compose.onNodeWithTag("history-version-$revision").performScrollTo().assertIsEnabled().performClick()
        document.await { state.metadataHistory.view?.revision == revision && !state.metadataHistory.busy }
        document.waitFor("metadata-saved-snapshot")
    }
    private fun selectParent(group: String?) {
        compose.onNodeWithTag("metadata-parent").performScrollTo().performClick()
        compose.onNodeWithTag(group?.let { "metadata-parent-$it" } ?: "metadata-parent-root").performScrollTo().performClick()
    }
    private fun hideIme() {
        compose.runOnIdle {
            compose.activity.currentFocus?.clearFocus()
            compose.activity.getSystemService(InputMethodManager::class.java)
                .hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0)
        }
        compose.waitUntil(10_000) {
            compose.activity.window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.ime()) == false
        }
    }
    private fun navigationTouch(tag: String) {
        hideIme()
        compose.onNodeWithTag(tag).assertIsDisplayed().assertIsEnabled().performTouchInput { click() }
    }
    private fun awaitSavedProjection() {
        try { document.await { state.metadataHistory.view != null && !state.metadataHistory.busy && state.metadata == null } }
        catch (timeout: ComposeTimeoutException) {
            var flags = ""
            compose.runOnIdle {
                flags = "mode=${state.metadataHistory.mode}, showing=${state.metadataHistory.showing}, selected=${state.metadataHistory.selection != null}, " +
                    "view=${state.metadataHistory.view != null}, busy=${state.metadataHistory.busy}, historyError=${state.metadataHistory.error}, " +
                    "editor=${state.metadata != null}, form=${state.activeForm != null}, navigating=${state.navigating}, " +
                    "decision=${state.navigationDecision}, invalid=${state.invalidInput}, failed=${state.error}, save=${state.saveLabel}"
            }
            throw AssertionError("PUBLIC history Save did not produce its selected projection; $flags", timeout)
        }
    }
    private fun confirmRestore(expected: () -> Boolean) {
        document.await { state.metadataHistory.mode == MetadataHistoryMode.RESTORE &&
            state.metadataHistory.view != null && !state.metadataHistory.busy && !state.editingBlocked }
        compose.onNodeWithTag("confirm-metadata-restore").assertIsDisplayed().assertIsEnabled()
            .performTouchInput { click() }
        try {
            document.await { state.metadataHistory.busy || state.metadataHistory.request()?.submitted == true ||
                state.metadataHistory.selection == null || state.metadataHistory.error }
            document.await { state.metadataHistory.selection == null && expected() }
        } catch (timeout: ComposeTimeoutException) {
            var flags = ""
            compose.runOnIdle {
                flags = "mode=${state.metadataHistory.mode}, showing=${state.metadataHistory.showing}, " +
                    "selected=${state.metadataHistory.selection != null}, view=${state.metadataHistory.view != null}, " +
                    "busy=${state.metadataHistory.busy}, blocked=${state.editingBlocked}, historyError=${state.metadataHistory.error}, " +
                    "retained=${state.metadataHistory.request() != null}, submitted=${state.metadataHistory.request()?.submitted == true}, " +
                    "navigating=${state.navigating}, decision=${state.navigationDecision}, invalid=${state.invalidInput}, failed=${state.error}"
            }
            throw AssertionError("PUBLIC metadata Restore did not acknowledge its exact action and projection; $flags", timeout)
        }
    }

    @Test fun groupViewCompareAndExplicitParentRestoreUseActualMetadataAndOneSameIdentityVersion() {
        val group = document.group("PUBLIC UI saved group metadata")
        val parent = document.group("PUBLIC UI restore parent")
        val revision = history(FormKind.GROUP, group).single().id
        compose.runOnIdle { state.dismissHistory(); state.beginMetadata(FormKind.GROUP, group) }
        document.await { state.metadata != null && !state.navigating }
        compose.runOnIdle { state.editMetadata(name = "PUBLIC UI current group metadata", description = "PUBLIC UI current description"); state.setGroupIcon("server"); state.saveNow() }
        document.await { state.metadata == null && state.activeForm == null && !state.navigating && state.overview?.groups?.any { it.id == group && it.name == "PUBLIC UI current group metadata" } == true }
        assertEquals(2, history(FormKind.GROUP, group).size)
        val before = document.savedBytes()
        view(revision)
        compose.onNodeWithTag("metadata-saved-snapshot").assert(hasAnyDescendant(hasText("PUBLIC UI saved group metadata")))
        compose.onNodeWithTag("entry-password").assertDoesNotExist()
        compose.onNodeWithTag("reveal-password").assertDoesNotExist()
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("compare-metadata-version").performScrollTo().performClick()
        document.waitFor("metadata-current-snapshot")
        compose.onNodeWithTag("metadata-current-snapshot").assert(hasAnyDescendant(hasText("PUBLIC UI current group metadata")))
        compose.onNodeWithTag("metadata-current-snapshot").assert(hasAnyDescendant(hasText("PUBLIC UI current description")))
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("restore-metadata-version").performScrollTo().performClick()
        document.waitFor("confirm-metadata-restore"); selectParent(parent)
        confirmRestore { state.overview?.groups?.any { it.id == group && it.name == "PUBLIC UI saved group metadata" && it.parent == parent } == true }
        assertFalse(before.contentEquals(document.savedBytes()))
        assertEquals(3, history(FormKind.GROUP, group).size)
        compose.runOnIdle { assertEquals(1, state.overview!!.groups.count { it.id == group }); state.dismissHistory() }
        document.reopen()
        compose.runOnIdle { assertEquals(parent, state.overview!!.groups.single { it.id == group }.parent); assertEquals("PUBLIC UI saved group metadata", state.overview!!.groups.single { it.id == group }.name) }
        assertEquals(3, history(FormKind.GROUP, group).size)
    }

    @Test fun databaseViewCompareRestoreHasNoParentOrEntryFieldsAndPreservesOtherContents() {
        val group = document.group("PUBLIC database history group")
        val entry = document.entry(group, PublicEntryFields(title = "PUBLIC database history entry", password = "PUBLIC preserved history password"))
        var originalName = ""
        compose.runOnIdle { originalName = state.overview!!.name }
        val revision = history(FormKind.DATABASE, null).single().id
        compose.runOnIdle { state.dismissHistory(); state.beginMetadata(FormKind.DATABASE) }
        document.await { state.metadata != null && !state.navigating }
        compose.runOnIdle { state.editMetadata(name = "PUBLIC UI current database metadata", description = "PUBLIC UI database description"); state.saveNow() }
        document.await { state.metadata == null && state.activeForm == null && !state.navigating && state.overview?.name == "PUBLIC UI current database metadata" }
        assertEquals(2, history(FormKind.DATABASE, null).size)
        val before = document.savedBytes()
        view(revision)
        compose.onNodeWithTag("metadata-saved-snapshot").assert(hasAnyDescendant(hasText(originalName)))
        compose.onNodeWithTag("metadata-parent").assertDoesNotExist()
        compose.onNodeWithTag("entry-password").assertDoesNotExist()
        compose.onNodeWithTag("compare-metadata-version").performScrollTo().performClick()
        compose.onNodeWithTag("metadata-current-snapshot").assert(hasAnyDescendant(hasText("PUBLIC UI current database metadata")))
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("restore-metadata-version").performScrollTo().performClick()
        compose.onNodeWithTag("metadata-parent").assertDoesNotExist()
        confirmRestore { state.overview?.name == originalName }
        compose.runOnIdle {
            assertEquals(listOf(group), state.overview!!.groups.map { it.id })
            assertEquals(listOf(entry), state.overview!!.entries.map { it.id })
            assertEquals("PUBLIC database history entry", state.overview!!.entries.single().title)
        }
        assertEquals(3, history(FormKind.DATABASE, null).size)
        compose.runOnIdle { state.dismissHistory() }
        document.reopen()
        compose.runOnIdle { assertEquals(originalName, state.overview!!.name) }
        assertEquals(3, history(FormKind.DATABASE, null).size)
        compose.runOnIdle { state.dismissHistory() }
        document.inspect(group, entry)
        compose.onNodeWithTag("reveal-password").performClick()
        document.await { state.revealed == "PUBLIC preserved history password" }
    }

    @Test fun dirtyEditorStayThenNavigationSavePrecedesReadAndExplicitRestore() {
        val group = document.group("PUBLIC original dirty history group")
        val revision = history(FormKind.GROUP, group).single().id
        compose.runOnIdle { state.dismissHistory(); state.beginMetadata(FormKind.GROUP, group) }
        document.await { state.metadata != null && !state.navigating }
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC unsaved history group")
        document.await { state.activeForm?.dirty == true && !state.editingBlocked }
        hideIme()
        val before = document.savedBytes()
        compose.runOnIdle { state.metadataHistory.open(FormKind.GROUP, group, revision) }
        document.waitFor("unsaved-changes")
        navigationTouch("navigation-stay")
        document.waitAbsent("unsaved-changes")
        compose.runOnIdle { assertNotNull(state.metadata); assertNull(state.metadataHistory.selection); assertEquals("PUBLIC unsaved history group", state.metadata!!.name) }
        assertArrayEquals(before, document.savedBytes())
        compose.runOnIdle { state.metadataHistory.open(FormKind.GROUP, group, revision) }
        document.waitFor("unsaved-changes")
        compose.runOnIdle { assertTrue(state.navigationDecision); assertFalse(state.navigating); assertFalse(state.error); assertFalse(state.invalidInput) }
        navigationTouch("navigation-save")
        document.await { state.navigating || state.metadataHistory.showing || state.invalidInput || state.error }
        awaitSavedProjection()
        compose.runOnIdle { assertEquals("PUBLIC unsaved history group", state.metadataHistory.view!!.current.name) }
        assertFalse(before.contentEquals(document.savedBytes()))
        compose.onNodeWithTag("restore-metadata-version").performScrollTo().performClick()
        confirmRestore { state.overview?.groups?.any { it.id == group && it.name == "PUBLIC original dirty history group" } == true }
        assertEquals(3, history(FormKind.GROUP, group).size)
    }

    @Test fun actualWorkerRestoreStaysDisabledUntilTheOwningProjectionGateIsReleased() {
        val group = document.group("PUBLIC gated original metadata")
        val revision = history(FormKind.GROUP, group).single().id
        compose.runOnIdle { state.dismissHistory(); state.beginMetadata(FormKind.GROUP, group) }
        document.await { state.metadata != null && !state.navigating }
        compose.runOnIdle { state.editMetadata(name = "PUBLIC gated current metadata"); state.saveNow() }
        document.await { state.activeForm == null && state.metadata == null && !state.navigating &&
            state.overview?.groups?.any { it.id == group && it.name == "PUBLIC gated current metadata" } == true }
        assertEquals(2, history(FormKind.GROUP, group).size)
        view(revision)
        compose.onNodeWithTag("restore-metadata-version").performScrollTo().performClick()
        document.await { state.metadataHistory.mode == MetadataHistoryMode.RESTORE && !state.editingBlocked }
        val before = document.savedBytes()
        var heldGate by mutableStateOf(true)
        // Control only the presentation gate. Every projection and confirmation still uses the
        // same actual MainActivity worker, native archive and retained MetadataHistoryState.
        compose.activityRule.scenario.onActivity { activity -> activity.setContent { TaypeerTheme {
            val attachments = rememberAttachmentActions(state)
            if (state.metadataHistory.showing) MetadataHistoryScreen(state.metadataHistory, true, {}, blocked = heldGate)
            else LibraryScreen(state, attachments, copySecret = { false }) {}
        } } }
        compose.onNodeWithTag("confirm-metadata-restore").assertIsDisplayed().assertIsNotEnabled()
            .performTouchInput { click() }
        compose.runOnIdle {
            assertNull(state.metadataHistory.request())
            assertEquals("PUBLIC gated current metadata", state.overview!!.groups.single { it.id == group }.name)
            assertEquals(group, state.metadataHistory.selection!!.target)
        }
        assertArrayEquals(before, document.savedBytes())
        compose.runOnIdle { heldGate = false }
        confirmRestore { state.overview?.groups?.any { it.id == group && it.name == "PUBLIC gated original metadata" } == true }
        assertFalse(before.contentEquals(document.savedBytes()))
        assertEquals(3, history(FormKind.GROUP, group).size)
        compose.runOnIdle { assertEquals(1, state.overview!!.groups.count { it.id == group }); state.dismissHistory() }
        document.reopen()
        compose.runOnIdle { assertEquals("PUBLIC gated original metadata", state.overview!!.groups.single { it.id == group }.name) }
        assertEquals(3, history(FormKind.GROUP, group).size)
    }
}
