package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import dev.taypeer.platform.WorkingCopies
import dev.taypeer.platform.anonymousCiphertext
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** Real isolated document generations, private pipes and descriptor-backed encrypted persistence. */
@RunWith(AndroidJUnit4::class)
class DocumentTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val password = "PUBLIC Android document master"
    private inline fun fixture(block: (Host, DocumentManager) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-document-${UUID.randomUUID()}")
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                DocumentManager(context, host).use { manager -> block(host, manager) }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    @Test fun twoRealDocumentsSaveReopenAndLockIndependently() = fixture { _, manager ->
        val first = manager.create("PUBLIC first", null, "PUBLIC first create", password)
        val second = manager.create("PUBLIC second", null, "PUBLIC second create", password)
        assertNotEquals(manager.process(first).pid, manager.process(second).pid)
        assertNotEquals(android.os.Process.myPid(), manager.process(first).pid)
        assertNotEquals(first.generation(), second.generation())
        val database = first.databaseId()
        first.beginEntry(null, null)
        first.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC durable entry"))
        val editor = first.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC protected sentinel"))
        val receipt = first.save(editor.form.draft, editor.form.revision, "PUBLIC entry snapshot")
        assertEquals(SaveState.SAVED, receipt.state)
        assertEquals(receipt, first.save(editor.form.draft, editor.form.revision, "PUBLIC entry snapshot"))
        assertEquals(1, first.overview("").entries.size)
        assertTrue(second.overview("").entries.isEmpty())
        val id = first.overview("").entries.single().id
        assertNull(first.entry(id).group)
        assertEquals(1, first.history(FormKind.ENTRY, id).size)
        first.lock()
        assertThrows(AndroidException::class.java) { first.overview("") }
        assertTrue(second.overview("").entries.isEmpty())
        manager.close(first)
        val path = manager.catalog().single { it.database == database }.path
        val reopened = manager.open(path, password)
        assertEquals("PUBLIC durable entry", reopened.entry(id).title)
        assertEquals(true, reopened.entry(id).hasPassword)
        assertEquals(1, reopened.history(FormKind.ENTRY, id).size)
    }

    @Test fun confirmedFilesAndIndependentIncompleteFormsSurviveProcessRestart() = fixture { _, manager ->
        val session = manager.create("PUBLIC durable", null, "PUBLIC create", password)
        val database = session.databaseId()
        session.beginEntry(null, null)
        val valid = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC saved"))
        session.save(valid.form.draft, valid.form.revision, "PUBLIC saved snapshot")
        val entry = session.overview("").entries.single().id
        session.beginEntry(entry, null)
        val incomplete = session.patchEntry(EntryTextField.TITLE, TextEdit.Set(""))
        assertEquals(SaveState.LOCAL_DRAFT_SAVED, session.save(incomplete.form.draft, incomplete.form.revision, "PUBLIC incomplete" ).state)
        session.beginEntry(null, null)
        val other = session.patchEntry(EntryTextField.USERNAME, TextEdit.Set("PUBLIC unfinished"))
        session.persistForms()
        assertEquals(2, session.forms().size)
        val path = manager.catalog().single { it.database == database }.path
        val ciphertext = File(path).readBytes()
        assertFalse(ciphertext.toString(Charsets.ISO_8859_1).contains("PUBLIC saved"))
        manager.close(session)
        assertThrows(AndroidException::class.java) { manager.open(path, "PUBLIC wrong password") }
        assertArrayEquals(ciphertext, File(path).readBytes())
        val reopened = manager.open(path, password)
        assertEquals("PUBLIC saved", reopened.entry(entry).title)
        val continued = reopened.beginEntry(entry, null)
        assertEquals(incomplete.form.draft, continued.form.draft)
        assertEquals("", continued.title)
        assertEquals(2, reopened.forms().size)
        reopened.deleteForm(other.form.draft)
        assertEquals(1, reopened.forms().size)
    }

    @Test fun importedCopyStaysReadOnlyAndPreservesItsSource() = fixture { host, manager ->
        val bytes = instrumentation.context.assets.open("populated.taypeer").use { it.readBytes() }
        val source = File(context.cacheDir, "PUBLIC-source-${UUID.randomUUID()}.taypeer")
        try {
            source.writeBytes(bytes)
            val copies = WorkingCopies(context, host)
            val working = copies.importStream { source.inputStream() }
            assertArrayEquals(bytes, source.readBytes())
            assertTrue(copies.catalog().isEmpty())
            val session = manager.open(working.path, "PUBLIC_SESSION_DRAFT_PASSWORD")
            assertFalse(session.overview("").writable)
            assertEquals(listOf(working.canonicalFile), copies.catalog().map { it.canonicalFile })
            assertThrows(AndroidException::class.java) { session.beginEntry(null, null) }
            assertArrayEquals(bytes, source.readBytes())
            assertArrayEquals(bytes, working.readBytes())
        } finally { assertTrue(source.delete()) }
    }

    @Test fun repeatedCreationUsesTheSameCopyAndRejectsChangedIntent() = fixture { _, manager ->
        val first = manager.create("PUBLIC exact creation", "PUBLIC description", "PUBLIC repeat create", password)
        val database = first.databaseId()
        val copy = manager.catalog().single()
        val confirmed = File(copy.path).readBytes()
        manager.close(first)
        val repeated = manager.create("PUBLIC exact creation", "PUBLIC description", "PUBLIC repeat create", password)
        assertEquals(database, repeated.databaseId())
        assertEquals(listOf(copy), manager.catalog())
        assertArrayEquals(confirmed, File(copy.path).readBytes())
        manager.close(repeated)
        assertThrows(AndroidException::class.java) {
            manager.create("PUBLIC changed intent", "PUBLIC description", "PUBLIC repeat create", password)
        }
        assertEquals(listOf(copy), manager.catalog())
        assertArrayEquals(confirmed, File(copy.path).readBytes())
    }

    @Test fun failedAllocatorRetainsTheFormAndBackgroundRevocationDoesNotWaitForBusyIo() {
        val profile = File(context.noBackupFilesDir, "PUBLIC-fault-${UUID.randomUUID()}")
        val fail = AtomicBoolean()
        val block = AtomicBoolean()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val command = Executors.newSingleThreadExecutor()
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                val manager = DocumentManager(context, host) {
                    if (fail.get()) throw AndroidException.StorageIo()
                    if (block.get()) { entered.countDown(); check(release.await(15, TimeUnit.SECONDS)) }
                    anonymousCiphertext(context)
                }
                try {
                    val session = manager.create("PUBLIC faults", null, "PUBLIC create", password)
                    session.beginEntry(null, null)
                    val editor = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC retained"))
                    fail.set(true)
                    assertThrows(AndroidException::class.java) { session.save(editor.form.draft, editor.form.revision, "PUBLIC retry" ) }
                    fail.set(false)
                    assertEquals("PUBLIC retained", session.editor().title)
                    assertTrue(session.overview("").entries.isEmpty())
                    assertEquals(SaveState.SAVED, session.save(editor.form.draft, editor.form.revision, "PUBLIC retry").state)
                    val changed = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC blocked"))
                    block.set(true)
                    val pending = command.submit { session.save(changed.form.draft, changed.form.revision, "PUBLIC busy" ) }
                    assertTrue(entered.await(10, TimeUnit.SECONDS))
                    val started = System.nanoTime()
                    manager.background()
                    assertTrue(TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - started) < 500)
                    assertEquals(dev.taypeer.bridge.SessionAccess.REVOKED, session.access())
                    val until = started + TimeUnit.SECONDS.toNanos(4)
                    while (!manager.process(session).exited() && System.nanoTime() < until) Thread.sleep(10)
                    assertTrue(manager.process(session).exited())
                    val process = manager.process(session)
                    val requestMs = TimeUnit.NANOSECONDS.toMillis(process.terminationRequestedNanos - started)
                    val observedMs = TimeUnit.NANOSECONDS.toMillis(process.exitObservedNanos - started)
                    android.util.Log.i("PublicShutdownTiming", "request_ms=$requestMs observed_ms=$observedMs")
                    assertTrue("PUBLIC shutdown request=${requestMs}ms observed=${observedMs}ms", requestMs in 0..2200)
                    assertTrue(observedMs >= requestMs)
                    release.countDown()
                    assertThrows(java.util.concurrent.ExecutionException::class.java) { pending.get(5, TimeUnit.SECONDS) }
                } finally {
                    release.countDown()
                    try { manager.close() } catch (_: AndroidException) { /* The forced exit is deliberately unconfirmed. */ }
                }
            }
        } finally { release.countDown(); command.shutdownNow(); assertTrue(profile.deleteRecursively()) }
    }

    private fun cancelBeforeBoot(manager: DocumentManager, command: java.util.concurrent.ExecutorService, launch: () -> Unit) {
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        manager.beforeProcessStart = { entered.countDown(); check(release.await(15, TimeUnit.SECONDS)) }
        try {
            val pending = command.submit { launch() }
            assertTrue(entered.await(10, TimeUnit.SECONDS))
            assertEquals(1, manager.pendingLaunchCount)
            val started = System.nanoTime()
            manager.background()
            assertTrue(TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - started) < 500)
            release.countDown()
            assertThrows(java.util.concurrent.ExecutionException::class.java) { pending.get(5, TimeUnit.SECONDS) }
            assertEquals(0, manager.pendingLaunchCount)
        } finally { release.countDown(); manager.beforeProcessStart = null }
    }

    @Test fun pendingCreationAndReopenCannotBootAfterBackground() = fixture { _, manager ->
        val command = Executors.newSingleThreadExecutor()
        try {
            cancelBeforeBoot(manager, command) { manager.create("PUBLIC late creation", null, "PUBLIC late create", password) }
            assertTrue(manager.catalog().isEmpty())
            val session = manager.create("PUBLIC pending reopen", null, "PUBLIC actual create", password)
            val database = session.databaseId()
            session.beginEntry(null, null)
            val form = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC retained before reopen")).form
            session.save(form.draft, form.revision, "PUBLIC retained snapshot")
            manager.close(session)
            val path = manager.catalog().single { it.database == database }.path
            val before = File(path).readBytes()
            cancelBeforeBoot(manager, command) { manager.open(path, password) }
            assertArrayEquals(before, File(path).readBytes())
            assertEquals("PUBLIC retained before reopen", manager.open(path, password).overview("").entries.single().title)
        } finally { command.shutdownNow() }
    }

    @Test fun pendingInvitationProofCannotBootAfterBackground() = fixture { sourceHost, authority ->
        val recipientProfile = File(context.noBackupFilesDir, "PUBLIC-pending-proof-${UUID.randomUUID()}")
        val command = Executors.newSingleThreadExecutor()
        try {
            val session = authority.create("PUBLIC proof source", null, "PUBLIC proof create", password)
            sourceHost.startExchange()
            val code = session.createInvitation()
            Host(recipientProfile.path, KeystoreCredentials(context)).use { recipientHost ->
                DocumentManager(context, recipientHost).use { recipient ->
                    cancelBeforeBoot(recipient, command) { recipient.join(code) }
                    assertTrue(recipient.catalog().isEmpty())
                    assertTrue(recipient.pendingJoins().isEmpty())
                }
            }
            assertTrue(session.overview("").entries.isEmpty())
        } finally { sourceHost.stopExchange(); command.shutdownNow(); assertTrue(recipientProfile.deleteRecursively()) }
    }
}
