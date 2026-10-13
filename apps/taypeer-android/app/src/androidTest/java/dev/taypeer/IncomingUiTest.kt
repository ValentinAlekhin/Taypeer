package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.ExchangeScheduler
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.UUID

/** Real Iroh delivery must update the visible recipient without another UI action. */
class IncomingUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val password = "PUBLIC incoming UI master"

    private fun waitFor(tag: String) = compose.waitUntil(30_000) {
        compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
    }
    private fun visible(value: String) = compose.waitUntil(30_000) {
        compose.onAllNodes(hasText(value)).fetchSemanticsNodes().isNotEmpty()
    }
    private fun save(session: DocumentSession, entry: String, title: String) {
        session.beginEditEntry(entry)
        val changed = session.patchEntry(EntryTextField.TITLE, TextEdit.Set(title))
        val operation = newOperation()
        val historyBefore = session.history(FormKind.ENTRY, entry).size
        // Receipt can advance the archive fingerprint before automatic application.
        // Retry only that explicit optimistic-write failure, preserving exact intent.
        compose.waitUntil(30_000) {
            try {
                val saved = session.save(changed.form.draft, changed.form.revision, operation)
                assertEquals(SaveState.SAVED, saved.state)
                assertEquals(operation, saved.operation)
                true
            } catch (_: AndroidException.StorageChanged) {
                val retained = session.editor()
                assertEquals(changed.form.draft, retained.form.draft)
                assertEquals(changed.form.revision, retained.form.revision)
                assertEquals(title, retained.title)
                assertTrue(retained.form.dirty)
                assertEquals(historyBefore, session.history(FormKind.ENTRY, entry).size)
                false
            }
        }
        assertEquals(title, session.entry(entry).title)
        assertEquals(historyBefore + 1, session.history(FormKind.ENTRY, entry).size)
        session.deleteForm(changed.form.draft)
    }
    private fun replica(scenario: (DocumentSession, String) -> Unit) {
        val context = compose.activity.applicationContext
        val directory = File(context.noBackupFilesDir, "PUBLIC-incoming-ui-${UUID.randomUUID()}")
        try { Host(directory.path, KeystoreCredentials(context)).use { host ->
            host.setRelayPreference(RelayPreference.Disabled)
            DocumentManager(context, host).use { manager ->
                try {
                    val authority = manager.create("PUBLIC incoming UI", null, newOperation(), password)
                    val initial = beginPublicEntry(authority)
                    val ready = authority.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC initial incoming title"))
                    assertEquals(SaveState.SAVED, authority.save(ready.form.draft, ready.form.revision, newOperation()).state)
                    authority.deleteForm(ready.form.draft)
                    val entry = initial.form.target!!
                    val database = authority.databaseId()
                    host.startExchange()
                    val code = authority.createInvitation()
                    compose.runOnIdle { compose.activity.documentState.join(code) }
                    compose.waitUntil(30_000) { compose.activity.documentState.pending.any { it.database == database } }
                    var request = ""
                    compose.runOnIdle { request = compose.activity.documentState.pending.single { it.database == database }.request }
                    authority.approveInvitation(request)
                    compose.runOnIdle { compose.activity.documentState.resumeJoin(request) }
                    compose.waitUntil(30_000) { compose.activity.documentState.catalog.any { it.database == database } }
                    compose.runOnIdle {
                        val state = compose.activity.documentState
                        state.unlock(state.catalog.single { it.database == database }, password)
                    }
                    visible("PUBLIC initial incoming title")
                    scenario(authority, entry)
                } finally {
                    // Stop delivery before orderly teardown; this owns only the PUBLIC peer fixture.
                    host.stopExchange()
                    ExchangeScheduler.stopForeground(context)
                    compose.runOnIdle { compose.activity.documentState.lock() }
                }
            }
        } } finally { assertTrue(directory.deleteRecursively()) }
    }

    @Test fun remoteSaveRefreshesBothTheListAndTheOpenInspectorWithoutRecipientActions() = replica { authority, entry ->
        save(authority, entry, "PUBLIC remotely updated list title")
        // No recipient navigation, query or refresh between the remote Save and this assertion.
        visible("PUBLIC remotely updated list title")
        compose.onNodeWithTag("entry-$entry").performClick()
        waitFor("edit-entry")
        save(authority, entry, "PUBLIC remotely updated inspector title")
        compose.waitUntil(30_000) {
            compose.activity.documentState.inspectedEntry?.title == "PUBLIC remotely updated inspector title"
        }
        visible("PUBLIC remotely updated inspector title")
        compose.runOnIdle { assertNull(compose.activity.documentState.entry) }
    }

    @Test fun remoteApplicationRefreshesTheOverviewAndPreservesTheDirtyEditorAndItsIdentity() = replica { authority, entry ->
        compose.onNodeWithTag("entry-$entry").performClick()
        waitFor("edit-entry")
        compose.onNodeWithTag("edit-entry").performClick()
        waitFor("entry-title")
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC recipient unsaved title")
        compose.waitUntil(30_000) { compose.activity.documentState.activeForm?.dirty == true }
        var draft = ""
        compose.runOnIdle { draft = compose.activity.documentState.activeForm!!.draft }
        save(authority, entry, "PUBLIC concurrent remote saved title")
        compose.waitUntil(30_000) {
            compose.activity.documentState.overview?.entries?.singleOrNull { it.id == entry }?.title == "PUBLIC concurrent remote saved title"
        }
        compose.onNodeWithTag("entry-title").assertTextEquals("PUBLIC recipient unsaved title")
        compose.runOnIdle {
            assertEquals(draft, compose.activity.documentState.activeForm!!.draft)
            assertTrue(compose.activity.documentState.activeForm!!.dirty)
        }
        compose.onNodeWithTag("cancel-form").performClick()
        waitFor("edit-entry")
        visible("PUBLIC concurrent remote saved title")
    }
}
