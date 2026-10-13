package dev.taypeer

import android.graphics.BitmapFactory
import android.graphics.Color
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

/** Real isolated workers; the queue controls only delivery after the actual command has returned. */
@RunWith(AndroidJUnit4::class)
class MetadataHistoryTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC metadata history master"
    private val svg = """<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect width="32" height="16" fill="#14dc28"/></svg>""".toByteArray()

    private class QueuedCommands(var session: DocumentSession) : SessionCommands {
        private data class Reply(val complete: () -> Unit, val fail: () -> Unit, val discard: () -> Unit)
        private val replies = ArrayDeque<Reply>()
        var executions = 0; private set
        var rejected = 0; private set
        override fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
            completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit, discarded: (T) -> Unit): Boolean {
            started(); executions++
            try {
                val actual = command(session)
                replies += Reply({ completed(actual) }, failed, { discarded(actual) })
            } catch (failure: AndroidException) {
                if (failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions) {
                    this.rejected++; replies += Reply(rejected, failed, {})
                } else replies += Reply(failed, failed, {})
            }
            return true
        }
        fun deliver() { val reply = replies.removeFirst(); try { reply.complete() } catch (_: Exception) { reply.fail() } }
        fun drop() { val reply = replies.removeFirst(); reply.discard(); reply.fail() }
    }
    private fun fixture(block: (DocumentManager, DocumentSession, String) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-metadata-history-${UUID.randomUUID()}")
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC saved database metadata", "PUBLIC saved database description", newOperation(), password)
                    assertNotEquals(android.os.Process.myPid(), manager.process(session).pid)
                    block(manager, session, manager.catalog().single().path)
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }
    private fun group(session: DocumentSession, name: String, parent: String? = null): String {
        val form = session.beginMetadata(FormKind.NEW_GROUP, null, parent)
        val edited = session.patchMetadata(form.form.draft, FormKind.NEW_GROUP, TextEdit.Set(name), TextEdit.Keep)
        val saved = session.save(edited.form.draft, edited.form.revision, newOperation())
        assertEquals(SaveState.SAVED, saved.state); session.deleteForm(saved.form.draft)
        return saved.form.target!!
    }
    private fun changeGroup(session: DocumentSession, group: String, name: String, description: String?, icon: String? = null) {
        val form = session.beginMetadata(FormKind.GROUP, group, null)
        session.patchMetadata(form.form.draft, FormKind.GROUP, TextEdit.Set(name), description?.let { TextEdit.Set(it) } ?: TextEdit.Clear)
        val edited = session.patchGroupIcon(form.form.draft, icon?.let { TextEdit.Set(it) } ?: TextEdit.Clear)
        assertEquals(SaveState.SAVED, session.save(edited.form.draft, edited.form.revision, newOperation()).state)
        session.deleteForm(edited.form.draft)
    }
    private fun changeDatabase(session: DocumentSession, name: String, description: String?) {
        val form = session.beginMetadata(FormKind.DATABASE, null, null)
        val edited = session.patchMetadata(form.form.draft, FormKind.DATABASE, TextEdit.Set(name), description?.let { TextEdit.Set(it) } ?: TextEdit.Clear)
        assertEquals(SaveState.SAVED, session.save(edited.form.draft, edited.form.revision, newOperation()).state)
        session.deleteForm(edited.form.draft)
    }

    @Test fun originalGroupImageAndCurrentLucideCompareWithoutNetworkWritesOrArtificialEntryFields() = fixture { _, session, path ->
        val form = session.beginMetadata(FormKind.NEW_GROUP, null, null)
        session.patchMetadata(form.form.draft, FormKind.NEW_GROUP, TextEdit.Set("PUBLIC saved image group"), TextEdit.Set("  PUBLIC saved description\nТекст  "))
        PublicIconServer(svg).use { server ->
            val imported = session.downloadIcon(form.form.draft, IconDownload.ImageUrl(server.url), newOperation())
            val saved = session.save(imported.draft, imported.revision, newOperation())
            val group = saved.form.target!!; session.deleteForm(saved.form.draft)
            val revision = session.history(FormKind.GROUP, group).single().id
            assertEquals(1, server.requests.get()); server.close()
            changeGroup(session, group, "PUBLIC current Lucide group", null, "folder")
            val before = File(path).readBytes()
            val commands = QueuedCommands(session)
            val state = MetadataHistoryState(commands, { session.databaseId() }) { _, _ -> fail("Viewing must not restore") }
            state.open(FormKind.GROUP, group, revision)
            assertTrue(state.busy); assertNull(state.view)
            commands.deliver()
            val view = state.view!!
            assertEquals("PUBLIC saved image group", view.saved.name)
            assertEquals("  PUBLIC saved description\nТекст  ", view.saved.description)
            assertEquals("PUBLIC current Lucide group", view.current.name)
            assertNull(view.current.description)
            assertEquals(IconView.Image, view.saved.icon)
            assertEquals(IconView.Lucide("folder"), view.current.icon)
            assertNull(state.currentPreview)
            val preview = state.savedPreview!!
            val image = BitmapFactory.decodeByteArray(preview.png, 0, preview.png.size)
            try { assertEquals(32, image.width); assertEquals(16, image.height); assertEquals(Color.rgb(20, 220, 40), image.getPixel(8, 8)) }
            finally { image.recycle() }
            state.compare(); assertEquals(MetadataHistoryMode.COMPARE, state.mode)
            assertTrue(session.forms().isEmpty()); assertEquals(2, session.history(FormKind.GROUP, group).size)
            assertArrayEquals(before, File(path).readBytes()); assertEquals(1, server.requests.get())
            state.close(); assertNull(state.view); assertNull(state.savedPreview)
            assertTrue(preview.png.all { it == 0.toByte() })
        }
    }

    @Test fun groupLostReplyRetainsExactBasisAndParentThroughBackLockSourcePurgeAndLaterMove() = fixture { manager, session, path ->
        val source = group(session, "PUBLIC original group")
        val parent = group(session, "PUBLIC explicit parent")
        val laterParent = group(session, "PUBLIC later parent")
        val revision = session.history(FormKind.GROUP, source).single().id
        changeGroup(session, source, "PUBLIC current group", "PUBLIC current description", "folder")
        val commands = QueuedCommands(session)
        var database: String? = session.databaseId()
        val restored = mutableListOf<Pair<FormKind, String>>()
        val state = MetadataHistoryState(commands, { database }) { kind, id -> restored += kind to id }
        state.open(FormKind.GROUP, source, revision); commands.deliver(); state.prepareRestore()
        val before = File(path).readBytes(); val acknowledgements = mutableListOf<Boolean>()
        val operation = newOperation()
        state.confirm(parent, operation, acknowledgements::add)
        val request = state.request()!!
        assertTrue(state.busy); assertTrue(restored.isEmpty()); assertEquals(operation, request.operation)
        assertEquals(parent, session.overview("").groups.single { it.id == source }.parent)
        assertEquals("PUBLIC original group", session.overview("").groups.single { it.id == source }.name)
        assertEquals(3, session.history(FormKind.GROUP, source).size)
        assertFalse(before.contentEquals(File(path).readBytes()))
        commands.drop(); assertTrue(state.error); assertEquals(listOf(false), acknowledgements)
        state.close(); assertEquals(listOf(request), state.unresolvedRequests)
        session.moveGroup(source, laterParent, newOperation())
        changeGroup(session, source, "PUBLIC independent later name", "PUBLIC later description", "server")
        session.purgeHistory(FormKind.GROUP, source, listOf(revision), newOperation())
        val history = session.history(FormKind.GROUP, source).map { it.id }
        val committed = File(path).readBytes()
        session.lock(); manager.close(session); database = null; state.reset()
        commands.session = manager.open(path, password); database = commands.session.databaseId()
        assertEquals(listOf(request), state.unresolvedRequests)
        state.retry(request, acknowledgements::add)
        assertArrayEquals(committed, File(path).readBytes())
        assertEquals(history, commands.session.history(FormKind.GROUP, source).map { it.id })
        assertEquals(laterParent, commands.session.overview("").groups.single { it.id == source }.parent)
        assertEquals("PUBLIC independent later name", commands.session.overview("").groups.single { it.id == source }.name)
        commands.deliver()
        assertEquals(listOf(false, true), acknowledgements)
        assertEquals(listOf(FormKind.GROUP to source), restored)
        assertTrue(state.unresolvedRequests.isEmpty()); assertNull(state.view)
        state.retry(request); assertEquals(3, commands.executions)
        assertArrayEquals(committed, File(path).readBytes())
    }

    @Test fun databaseRestoreRetainsIdentityAndExactReceiptAfterLaterMetadataAndPurge() = fixture { manager, session, path ->
        // Managed creation confirms its initial name, then the optional description.
        val originals = session.history(FormKind.DATABASE, null)
        assertEquals(2, originals.size)
        val revision = originals.single { it.name == "PUBLIC saved database metadata" &&
            it.description == "PUBLIC saved database description" }.id
        changeDatabase(session, "PUBLIC current database metadata", null)
        val commands = QueuedCommands(session)
        var database: String? = session.databaseId()
        val state = MetadataHistoryState(commands, { database }) { kind, id -> assertEquals(FormKind.DATABASE, kind); assertEquals(database, id) }
        state.open(FormKind.DATABASE, null, revision); commands.deliver()
        assertNull(state.view!!.saved.icon); assertNull(state.view!!.saved.parent)
        assertEquals("PUBLIC current database metadata", state.view!!.current.name)
        state.prepareRestore(); state.confirm(null, newOperation())
        val request = state.request()!!
        assertEquals("PUBLIC saved database metadata", session.overview("").name)
        assertEquals("PUBLIC saved database description", session.overview("").description)
        val restoredHistory = session.history(FormKind.DATABASE, null)
        assertEquals(4, restoredHistory.size)
        assertTrue(restoredHistory.map { it.id }.containsAll(originals.map { it.id }))
        commands.drop(); state.close()
        changeDatabase(session, "PUBLIC later database metadata", "PUBLIC later description")
        session.purgeHistory(FormKind.DATABASE, null, listOf(revision), newOperation())
        val committed = File(path).readBytes(); val history = session.history(FormKind.DATABASE, null).map { it.id }
        session.lock(); manager.close(session); database = null; state.reset()
        commands.session = manager.open(path, password); database = commands.session.databaseId()
        state.retry(request); commands.deliver()
        assertArrayEquals(committed, File(path).readBytes())
        assertEquals("PUBLIC later database metadata", commands.session.overview("").name)
        assertEquals(history, commands.session.history(FormKind.DATABASE, null).map { it.id })
        assertTrue(state.unresolvedRequests.isEmpty())
    }

    @Test fun deletedDestinationRejectsAllMetadataAndAllowsCorrectedIntentWithoutHistoryOrByteChanges() = fixture { _, session, path ->
        val source = group(session, "PUBLIC original rejected group")
        val parent = group(session, "PUBLIC subsequently trashed parent")
        val revision = session.history(FormKind.GROUP, source).single().id
        changeGroup(session, source, "PUBLIC current rejected group", "PUBLIC current retained description", "server")
        val commands = QueuedCommands(session)
        val state = MetadataHistoryState(commands, { session.databaseId() }) { _, _ -> }
        state.open(FormKind.GROUP, source, revision); commands.deliver(); state.prepareRestore()
        session.trash(FormKind.GROUP, parent, newOperation())
        val before = File(path).readBytes(); val history = session.history(FormKind.GROUP, source).map { it.id }
        state.confirm(parent, newOperation()); assertEquals(1, commands.rejected); commands.deliver()
        assertTrue(state.error); assertNull(state.request()); assertTrue(state.unresolvedRequests.isEmpty())
        assertArrayEquals(before, File(path).readBytes())
        assertEquals(history, session.history(FormKind.GROUP, source).map { it.id })
        assertEquals("PUBLIC current rejected group", session.overview("").groups.single { it.id == source }.name)
        state.confirm(null, newOperation()); commands.deliver()
        assertEquals(3, session.history(FormKind.GROUP, source).size)
        assertEquals("PUBLIC original rejected group", session.overview("").groups.single { it.id == source }.name)
        assertNull(session.overview("").groups.single { it.id == source }.parent)
    }

    @Test fun lateProjectionAfterCloseResetOrOtherDatabaseNeverReturnsSnapshotsAndOldIntentIsScoped() = fixture { manager, session, path ->
        val group = group(session, "PUBLIC delayed history group")
        val revision = session.history(FormKind.GROUP, group).single().id
        val commands = QueuedCommands(session)
        var database: String? = session.databaseId()
        val state = MetadataHistoryState(commands, { database }) { _, _ -> fail("Old result must not navigate") }
        val before = File(path).readBytes()
        state.open(FormKind.GROUP, group, revision); state.reset(); commands.deliver()
        assertNull(state.selection); assertNull(state.view); assertNull(state.savedPreview)
        assertArrayEquals(before, File(path).readBytes())
        state.open(FormKind.GROUP, group, revision); commands.deliver(); state.prepareRestore()
        state.confirm(null, newOperation()); val request = state.request()!!
        commands.drop(); state.close()
        val committed = File(path).readBytes()
        val other = manager.create("PUBLIC unrelated metadata history", null, newOperation(), password)
        database = other.databaseId(); commands.session = other; state.reset()
        val otherPath = manager.catalog().single { it.database == database }.path
        val otherBytes = File(otherPath).readBytes(); val count = commands.executions
        state.retry(request)
        assertEquals(count, commands.executions); assertTrue(state.unresolvedRequests.isEmpty())
        assertArrayEquals(otherBytes, File(otherPath).readBytes()); assertArrayEquals(committed, File(path).readBytes())
    }
}
