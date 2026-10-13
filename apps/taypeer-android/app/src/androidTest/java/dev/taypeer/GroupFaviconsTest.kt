package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Real isolated Rust selection, explicit host HTTP and independently durable per-entry receipts. */
@RunWith(AndroidJUnit4::class)
class GroupFaviconsTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC group favicon master"
    private val image = """<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#14dc28"/></svg>""".toByteArray()

    private inline fun fixture(block: (DocumentSession, DocumentManager) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-group-favicons-${UUID.randomUUID()}")
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                DocumentManager(context, host).use { manager ->
                    block(manager.create("PUBLIC group favicon database", null, newOperation(), password), manager)
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }
    private fun entry(session: DocumentSession, group: String, title: String, url: String?, icon: String? = null): String {
        session.beginCreateEntry(group)
        session.patchEntry(EntryTextField.TITLE, TextEdit.Set(title))
        session.patchEntry(EntryTextField.USERNAME, TextEdit.Set("PUBLIC preserved username"))
        session.patchEntry(EntryTextField.URL, url?.let(TextEdit::Set) ?: TextEdit.Clear)
        if (icon != null) session.setIcon(icon, newOperation())
        val ready = session.editor()
        assertEquals(SaveState.SAVED, session.save(ready.form.draft, ready.form.revision, newOperation()).state)
        session.deleteForm(ready.form.draft)
        return ready.form.target!!
    }

    @Test fun explicitPartialBatchReportsActualRowsAndReplayNeverRollsBackLaterSavedFields() = fixture { session, manager ->
        val group = publicGroup(session)
        PublicFaviconServer(image).use { server ->
            val good = entry(session, group, "PUBLIC acquired", server.url)
            val rejected = entry(session, group, "PUBLIC malformed URL", "PUBLIC invalid URL")
            val missing = entry(session, group, "PUBLIC no URL", null)
            val custom = entry(session, group, "PUBLIC custom", server.url, "key-round")
            val path = manager.catalog().single().path
            session.prepareGroupFavicons(group, false, false, newOperation()).use { batch ->
                assertEquals(0, server.requests.get())
                assertEquals(GroupFaviconState.SKIPPED, batch.rows().single { it.entry == custom }.state)
                assertEquals(GroupFaviconState.MISSING_URL, batch.rows().single { it.entry == missing }.state)
                val result = batch.run()
                assertEquals(GroupFaviconState.APPLIED, result.single { it.entry == good }.state)
                assertEquals(GroupFaviconState.FAILED, result.single { it.entry == rejected }.state)
                assertEquals(GroupFaviconFailure.Image(IconFailure.INVALID_URL), result.single { it.entry == rejected }.failure)
                assertEquals(2, server.requests.get())
                assertEquals(listOf("/PUBLIC-page", "/PUBLIC.svg"), server.requestPaths())
                assertEquals(IconView.Image, session.entry(good).icon)
                assertEquals("PUBLIC preserved username", session.entry(good).username)
                assertEquals(2, session.history(FormKind.ENTRY, good).size)
                assertEquals(1, session.history(FormKind.ENTRY, rejected).size)

                session.beginEditEntry(good)
                session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC later saved title"))
                session.setIcon("server", newOperation())
                val later = session.editor()
                session.save(later.form.draft, later.form.revision, newOperation())
                session.deleteForm(later.form.draft)
                val saved = File(path).readBytes()
                val again = batch.run()
                assertEquals(GroupFaviconState.APPLIED, again.single { it.entry == good }.state)
                assertEquals(2, server.requests.get())
                assertEquals(listOf("/PUBLIC-page", "/PUBLIC.svg"), server.requestPaths())
                assertEquals("PUBLIC later saved title", session.entry(good).title)
                assertEquals(IconView.Lucide("server"), session.entry(good).icon)
                assertEquals(3, session.history(FormKind.ENTRY, good).size)
                assertArrayEquals(saved, File(path).readBytes())
            }
        }
    }

    @Test fun cancellingThenLockingDuringActualHttpCannotCommitALateIconOrReviveTheBatch() = fixture { session, manager ->
        val group = publicGroup(session)
        PublicFaviconServer(image, holdImage = true).use { server ->
            val entry = entry(session, group, "PUBLIC cancelled network", server.url)
            session.prepareGroupFavicons(group, false, false, newOperation()).use { batch ->
                val path = manager.catalog().single().path
                val saved = File(path).readBytes()
                val executor = Executors.newSingleThreadExecutor()
                try {
                    val running = executor.submit<List<GroupFaviconRow>> { batch.run() }
                    assertTrue(server.entered.await(10, TimeUnit.SECONDS))
                    assertEquals(listOf("/PUBLIC-page", "/PUBLIC.svg"), server.requestPaths())
                    batch.cancel()
                    session.lock()
                    server.release.countDown()
                    val refused = assertThrows(java.util.concurrent.ExecutionException::class.java) { running.get(15, TimeUnit.SECONDS) }
                    assertTrue(refused.cause is AndroidException.Runtime)
                    assertEquals(GroupFaviconState.CANCELLED, batch.rows().single().state)
                    assertThrows(AndroidException.Runtime::class.java) { batch.run() }
                    assertArrayEquals(saved, File(path).readBytes())
                    manager.close(session)
                    val reopened = manager.open(path, password)
                    assertEquals(IconView.Default, reopened.entry(entry).icon)
                    assertEquals(1, reopened.history(FormKind.ENTRY, entry).size)
                } finally { server.release.countDown(); executor.shutdownNow() }
            }
        }
    }
}
