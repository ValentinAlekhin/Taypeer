package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.RelayPreference
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Real isolated workers exercise typed lifecycle capabilities and durable shared receipts. */
@RunWith(AndroidJUnit4::class)
class LifecycleTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC lifecycle master"
    private fun fixture(scenario: (DocumentManager) -> Unit) {
        val directory = File(context.noBackupFilesDir, "PUBLIC-lifecycle-${UUID.randomUUID()}")
        try {
            Host(directory.path, KeystoreCredentials(context)).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use(scenario)
            }
        } finally { assertTrue(directory.deleteRecursively()) }
    }
    private fun group(session: DocumentSession, name: String, parent: String? = null): String {
        val initial = session.beginMetadata(FormKind.NEW_GROUP, null, parent)
        val ready = session.patchMetadata(initial.form.draft, FormKind.NEW_GROUP, TextEdit.Set(name), TextEdit.Keep)
        val saved = session.save(ready.form.draft, ready.form.revision, newOperation())
        assertEquals(SaveState.SAVED, saved.state)
        session.deleteForm(saved.form.draft)
        return saved.form.target!!
    }
    private fun entry(session: DocumentSession, group: String): String {
        session.beginCreateEntry(group)
        session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC retained entry"))
        val ready = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC lifecycle protected value"))
        val saved = session.save(ready.form.draft, ready.form.revision, newOperation())
        assertEquals(SaveState.SAVED, saved.state)
        session.deleteForm(saved.form.draft)
        return saved.form.target!!
    }
    @Test fun cloneAndMoveUseSharedRulesAndDurableRetryReceipts() = fixture { manager ->
        val session = manager.create("PUBLIC lifecycle clones", null, newOperation(), password)
        val source = group(session, "PUBLIC source group")
        val destination = group(session, "PUBLIC destination group")
        val original = entry(session, source)
        val cloneOperation = newOperation()
        val copied = session.cloneEntry(original, destination, "PUBLIC copied entry", cloneOperation)
        assertEquals(copied, session.cloneEntry(original, destination, "PUBLIC copied entry", cloneOperation))
        assertNotEquals(original, copied)
        assertEquals(destination, session.entry(copied).group)
        assertEquals("PUBLIC lifecycle protected value", session.reveal(copied, null, null))
        assertEquals(1, session.history(FormKind.ENTRY, copied).size)
        val moveOperation = newOperation()
        session.moveEntry(original, destination, moveOperation)
        session.moveEntry(original, destination, moveOperation)
        assertEquals(destination, session.entry(original).group)
        val subtreeOperation = newOperation()
        val subtree = session.cloneGroup(destination, source, "PUBLIC copied subtree", subtreeOperation)
        assertEquals(subtree, session.cloneGroup(destination, source, "PUBLIC copied subtree", subtreeOperation))
        assertEquals(1, subtree.count { it.kind == FormKind.GROUP })
        assertEquals(2, subtree.count { it.kind == FormKind.ENTRY })
        val copiedGroup = subtree.single { it.kind == FormKind.GROUP }.target
        assertEquals(source, session.overview("").groups.single { it.id == copiedGroup }.parent)
        val groupMoveOperation = newOperation()
        session.moveGroup(copiedGroup, null, groupMoveOperation)
        session.moveGroup(copiedGroup, null, groupMoveOperation)
        assertNull(session.overview("").groups.single { it.id == copiedGroup }.parent)
        assertThrows(AndroidException::class.java) { session.moveGroup(source, source, newOperation()) }
        assertNull(session.overview("").groups.single { it.id == source }.parent)
        val file = manager.catalog().single().path
        manager.close(session)
        val reopened = manager.open(file, password)
        assertEquals(4, reopened.overview("").entries.size)
        assertEquals(destination, reopened.entry(original).group)
        assertEquals("PUBLIC lifecycle protected value", reopened.reveal(copied, null, null))
        assertEquals(1, reopened.history(FormKind.ENTRY, copied).size)
    }
    @Test fun reviewedRestoreAndPurgeAreExactRetryableAndBoundToTheLiveSession() = fixture { manager ->
        val session = manager.create("PUBLIC lifecycle reviews", null, newOperation(), password)
        val root = group(session, "PUBLIC trash root")
        val child = group(session, "PUBLIC trash child", root)
        val original = entry(session, child)
        session.trash(FormKind.GROUP, root, newOperation())
        val rows = session.trashRows()
        assertEquals(3, rows.size)
        assertEquals("PUBLIC retained entry", rows.single { it.target == original }.name)
        val rootRow = rows.single { it.target == root }
        assertThrows(AndroidException::class.java) { session.prepareTrashPurge(rootRow.kind, rootRow.target, "PUBLIC wrong generation") }
        val cancelled = session.prepareTrashRestore(rootRow.kind, rootRow.target, rootRow.generation, null)
        assertEquals(2u, cancelled.groups)
        assertEquals(1u, cancelled.entries)
        session.discardTrashReview(cancelled.review)
        assertThrows(AndroidException::class.java) { session.confirmTrashReview(cancelled.review, newOperation()) }
        assertEquals(3, session.trashRows().size)
        val restoration = session.prepareTrashRestore(rootRow.kind, rootRow.target, rootRow.generation, null)
        val restoreOperation = newOperation()
        session.confirmTrashReview(restoration.review, restoreOperation)
        session.confirmTrashReview(restoration.review, restoreOperation)
        assertTrue(session.trashRows().isEmpty())
        assertEquals(child, session.entry(original).group)
        assertThrows(AndroidException::class.java) { session.confirmTrashReview(restoration.review, newOperation()) }
        session.discardTrashReview(restoration.review)
        session.trash(FormKind.GROUP, root, newOperation())
        val reviewedRoot = session.trashRows().single { it.target == root }
        val purge = session.prepareTrashPurge(reviewedRoot.kind, root, reviewedRoot.generation)
        assertEquals(2u, purge.groups)
        assertEquals(1u, purge.entries)
        val purgeOperation = newOperation()
        session.confirmTrashReview(purge.review, purgeOperation)
        val path = manager.catalog().single().path
        val confirmed = File(path).readBytes()
        session.confirmTrashReview(purge.review, purgeOperation)
        assertArrayEquals(confirmed, File(path).readBytes())
        assertTrue(session.trashRows().isEmpty())
        assertThrows(AndroidException::class.java) { session.entry(original) }
        session.lock()
        assertThrows(AndroidException::class.java) { session.confirmTrashReview(purge.review, purgeOperation) }
        manager.close(session)
        val reopened = manager.open(path, password)
        assertTrue(reopened.trashRows().isEmpty())
        assertTrue(reopened.overview("").entries.isEmpty())
        assertThrows(AndroidException::class.java) { reopened.confirmTrashReview(purge.review, purgeOperation) }
    }
    @Test fun confirmingTheOriginalPurgeDoesNotReprepareAfterAnEntryWasRestored() = fixture { manager ->
        val session = manager.create("PUBLIC exact purge review", null, newOperation(), password)
        val retained = group(session, "PUBLIC retained group")
        val destination = group(session, "PUBLIC active destination")
        val original = entry(session, retained)
        session.trash(FormKind.GROUP, retained, newOperation())
        val rows = session.trashRows()
        val root = rows.single { it.target == retained }
        val reviewed = session.prepareTrashPurge(root.kind, root.target, root.generation)
        assertEquals(1u, reviewed.groups)
        assertEquals(1u, reviewed.entries)
        val child = rows.single { it.target == original }
        val restore = session.prepareTrashRestore(child.kind, child.target, child.generation, destination)
        session.confirmTrashReview(restore.review, newOperation())
        assertThrows(AndroidException::class.java) { session.confirmTrashReview(reviewed.review, newOperation()) }
        assertEquals(destination, session.entry(original).group)
        assertEquals(retained, session.trashRows().single().target)
        session.discardTrashReview(reviewed.review)
        val next = session.prepareTrashPurge(root.kind, root.target, root.generation)
        assertEquals(1u, next.groups)
        assertEquals(0u, next.entries)
        session.confirmTrashReview(next.review, newOperation())
        assertTrue(session.trashRows().isEmpty())
        assertEquals(destination, session.entry(original).group)
    }
}
