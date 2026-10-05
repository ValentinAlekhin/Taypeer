package dev.taypeer

import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.SelectedInput
import dev.taypeer.bridge.SelectedOutput
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import dev.taypeer.platform.SelectedDescriptors
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/** Selected SAF data crosses the actual private Binder port of a real document worker. */
@RunWith(AndroidJUnit4::class)
class AttachmentTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC selected attachment master"
    private inline fun fixture(allowUnconfirmedClose: Boolean = false, block: (DocumentManager, File) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-attachment-${UUID.randomUUID()}")
        val selected = File(context.cacheDir, "PUBLIC-selected-fixtures/PUBLIC-${UUID.randomUUID()}.bin")
        check(selected.parentFile!!.isDirectory || selected.parentFile!!.mkdirs())
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                val manager = DocumentManager(context, host)
                try { block(manager, selected) }
                finally {
                    try { manager.close() } catch (error: AndroidException) {
                        if (!allowUnconfirmedClose) throw error // Forced busy-worker exit cannot confirm close.
                    }
                }
            }
        } finally { selected.delete(); assertTrue(profile.deleteRecursively()) }
    }
    private fun uri(file: File) = Uri.parse("content://dev.taypeer.publicselected/${file.name}")
    private class CountingInput(private val delegate: SelectedInput) : SelectedInput {
        val reads = AtomicInteger()
        val closed = AtomicBoolean()
        override fun length() = delegate.length()
        override fun read(offset: ULong, count: UInt): ByteArray {
            assertTrue(count <= 65536u)
            reads.incrementAndGet()
            return delegate.read(offset, count)
        }
        override fun close() { closed.set(true); delegate.close() }
    }
    @Test fun selectedAttachmentSurvivesReopenAndRemovalRetainsExplicitHistoricalExport() = fixture { manager, source ->
        val public = ByteArray(131073) { (it % 251).toByte() }
        source.writeBytes(public)
        val session = manager.create("PUBLIC attachments", null, "PUBLIC attachments create", password)
        val database = session.databaseId()
        session.beginEntry(null, null)
        val initial = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC attached entry"))
        assertEquals(source.name, SelectedDescriptors.displayName(context, uri(source)))
        val input = CountingInput(SelectedDescriptors.openInput(context, uri(source)))
        val added = session.importAttachment(initial.form.draft, "PUBLIC selected.bin", null, "PUBLIC import", input)
        assertTrue(input.reads.get() >= 3)
        assertTrue(input.closed.get())
        val attachment = session.formAttachments().single()
        val blob = attachment.contents.single().blob
        assertEquals(public.size.toULong(), attachment.contents.single().bytes)
        val retry = CountingInput(SelectedDescriptors.openInput(context, uri(source)))
        session.importAttachment(initial.form.draft, "PUBLIC selected.bin", null, "PUBLIC import", retry)
        assertTrue(retry.reads.get() >= 3) // Exact retry verifies contents of a potentially mutable provider.
        assertTrue(retry.closed.get())
        assertEquals(listOf(attachment), session.formAttachments())
        val changed = public.copyOf().apply { this[0] = 42 }
        source.writeBytes(changed)
        val changedInput = CountingInput(SelectedDescriptors.openInput(context, uri(source)))
        assertThrows(AndroidException::class.java) {
            session.importAttachment(initial.form.draft, "PUBLIC selected.bin", null, "PUBLIC import", changedInput)
        }
        assertTrue(changedInput.closed.get())
        assertEquals(listOf(attachment), session.formAttachments())
        source.writeBytes(public)
        assertEquals(SaveState.SAVED, session.save(added.form.draft, added.form.revision, "PUBLIC attached snapshot").state)
        val entry = session.overview("").entries.single().id
        val original = session.history(FormKind.ENTRY, entry).single().id
        val renamed = session.renameAttachment(added.form.draft, attachment.attachment, "PUBLIC renamed.bin", "PUBLIC rename")
        session.save(renamed.form.draft, renamed.form.revision, "PUBLIC renamed snapshot")
        assertEquals("PUBLIC renamed.bin", session.entryAttachments(entry, null).single().name)
        val removed = session.removeAttachment(added.form.draft, attachment.attachment, "PUBLIC remove")
        session.save(removed.form.draft, removed.form.revision, "PUBLIC removed snapshot")
        val path = manager.catalog().single { it.database == database }.path
        assertFalse(File(path).readBytes().toString(Charsets.ISO_8859_1).contains("PUBLIC selected.bin"))
        manager.close(session)
        val reopened = manager.open(path, password)
        assertTrue(reopened.entryAttachments(entry, null).isEmpty())
        assertEquals("PUBLIC selected.bin", reopened.entryAttachments(entry, original).single().name)
        val output = File(source.parentFile, "PUBLIC-${UUID.randomUUID()}.out")
        try {
            output.writeBytes(ByteArray(public.size + 10) { 42 })
            val unchanged = output.readBytes()
            assertThrows(AndroidException::class.java) {
                reopened.exportAttachment(entry, null, blob, SelectedDescriptors.openOutput(context, uri(output)))
            }
            assertArrayEquals(unchanged, output.readBytes()) // Current entry no longer grants this blob.
            reopened.exportAttachment(entry, original, blob, SelectedDescriptors.openOutput(context, uri(output)))
            assertArrayEquals(public, output.readBytes())
            assertArrayEquals(public, source.readBytes())
        } finally { output.delete() }
    }

    @Test fun incompleteEncryptedFormRetainsSelectedBinaryAfterWorkerRestart() = fixture { manager, source ->
        source.writeText("PUBLIC local unfinished attachment")
        val session = manager.create("PUBLIC local attachments", null, "PUBLIC local create", password)
        val database = session.databaseId()
        val initial = session.beginEntry(null, null)
        val editor = session.importAttachment(initial.form.draft, "PUBLIC unfinished.bin", null, "PUBLIC local import", SelectedDescriptors.openInput(context, uri(source)))
        assertEquals(SaveState.LOCAL_DRAFT_SAVED, session.save(editor.form.draft, editor.form.revision, "PUBLIC local snapshot").state)
        val path = manager.catalog().single { it.database == database }.path
        manager.close(session)
        val reopened = manager.open(path, password)
        assertTrue(reopened.overview("").entries.isEmpty())
        reopened.resumeForm(initial.form.draft)
        assertEquals("PUBLIC unfinished.bin", reopened.formAttachments().single().name)
        val ready = reopened.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC completed"))
        assertEquals(SaveState.SAVED, reopened.save(ready.form.draft, ready.form.revision, "PUBLIC completed snapshot").state)
        val entry = reopened.overview("").entries.single().id
        val blob = reopened.entryAttachments(entry, null).single().contents.single().blob
        val output = File(source.parentFile, "PUBLIC-${UUID.randomUUID()}.out")
        try {
            reopened.exportAttachment(entry, null, blob, SelectedDescriptors.openOutput(context, uri(output)))
            assertArrayEquals(source.readBytes(), output.readBytes())
        } finally { output.delete() }
    }

    @Test fun limitIsCheckedBeforeReadingAndFailedReplacementPreservesOriginal() = fixture { manager, source ->
        source.writeText("PUBLIC retained attachment")
        val session = manager.create("PUBLIC failures", null, "PUBLIC failure create", password)
        session.beginEntry(null, null)
        val initial = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC retained"))
        session.importAttachment(initial.form.draft, "PUBLIC retained.bin", null, "PUBLIC initial import", SelectedDescriptors.openInput(context, uri(source)))
        val original = session.formAttachments().single()
        val closed = AtomicBoolean()
        val reads = AtomicInteger()
        val oversized = object : SelectedInput {
            override fun length() = 10uL * 1024uL * 1024uL + 1uL
            override fun read(offset: ULong, count: UInt): ByteArray { reads.incrementAndGet(); throw AndroidException.StorageIo() }
            override fun close() { closed.set(true) }
        }
        assertThrows(AndroidException::class.java) { session.importAttachment(initial.form.draft, "PUBLIC over limit", original.attachment, "PUBLIC oversized", oversized) }
        assertEquals(0, reads.get())
        assertTrue(closed.get())
        assertEquals(listOf(original), session.formAttachments())
        closed.set(false)
        val failed = object : SelectedInput {
            override fun length() = 32uL
            override fun read(offset: ULong, count: UInt): ByteArray = throw AndroidException.StorageIo()
            override fun close() { closed.set(true) }
        }
        assertThrows(AndroidException::class.java) { session.importAttachment(initial.form.draft, "PUBLIC provider failure", original.attachment, "PUBLIC failing replacement", failed) }
        assertTrue(closed.get())
        assertEquals(listOf(original), session.formAttachments())
        val staleSize = CountingInput(SelectedDescriptors.openInput(context, uri(source)))
        source.appendBytes(byteArrayOf(42)) // Its descriptor declared the previous, smaller length.
        assertThrows(AndroidException::class.java) {
            session.importAttachment(initial.form.draft, "PUBLIC changed size", original.attachment, "PUBLIC long provider", staleSize)
        }
        assertTrue(staleSize.closed.get())
        assertEquals(listOf(original), session.formAttachments())
        source.writeText("PUBLIC retained attachment")
        val editor = session.editor()
        session.save(editor.form.draft, editor.form.revision, "PUBLIC original snapshot")
        val entry = session.overview("").entries.single().id
        val outputClosed = AtomicBoolean()
        val output = object : SelectedOutput {
            override fun write(offset: ULong, bytes: ByteArray): UInt = bytes.size.toUInt().also { bytes.fill(0) }
            override fun finish(): Unit = throw AndroidException.StorageIo()
            override fun close() { outputClosed.set(true) }
        }
        assertThrows(AndroidException.StorageIo::class.java) { session.exportAttachment(entry, null, original.contents.single().blob, output) }
        assertTrue(outputClosed.get())
        assertEquals(listOf(original), session.entryAttachments(entry, null))
    }

    @Test fun backgroundRevocationDoesNotWaitForBlockedSelectedProvider() = fixture(allowUnconfirmedClose = true) { manager, _ ->
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val closed = AtomicBoolean()
        val command = Executors.newSingleThreadExecutor()
        try {
            val session = manager.create("PUBLIC busy selected", null, "PUBLIC busy create", password)
            session.beginEntry(null, null)
            val editor = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC busy entry"))
            val input = object : SelectedInput {
                override fun length() = 16uL
                override fun read(offset: ULong, count: UInt): ByteArray {
                    entered.countDown()
                    check(release.await(15, TimeUnit.SECONDS))
                    return ByteArray(count.toInt()) { 42 }
                }
                override fun close() { closed.set(true) }
            }
            val pending = command.submit { session.importAttachment(editor.form.draft, "PUBLIC blocked", null, "PUBLIC blocked import", input) }
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
            val closeUntil = System.nanoTime() + TimeUnit.SECONDS.toNanos(2)
            while (!closed.get() && System.nanoTime() < closeUntil) Thread.sleep(10)
            assertTrue(closed.get())
        } finally { release.countDown(); command.shutdownNow() }
    }
}
