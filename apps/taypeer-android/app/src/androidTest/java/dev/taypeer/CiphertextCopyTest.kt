package dev.taypeer

import android.net.Uri
import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.SelectedDescriptorOutput
import dev.taypeer.platform.SelectedDescriptors
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** Whole encrypted archives use genuine private SAF providers/PFDs and isolated Rust workers. */
@RunWith(AndroidJUnit4::class)
class CiphertextCopyTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val password = "PUBLIC whole encrypted copy master"
    private fun uri(file: File) = Uri.parse("content://dev.taypeer.publicselected/${file.name}")
    private fun destination(extension: String = "taypeer"): File {
        val directory = File(context.cacheDir, "PUBLIC-selected-fixtures")
        check(directory.isDirectory || directory.mkdirs())
        return File(directory, "PUBLIC-${UUID.randomUUID()}.$extension")
    }
    private inline fun fixture(block: (Host, DocumentManager, DocumentSession, WorkingCopyView) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-ciphertext-copy-${UUID.randomUUID()}")
        try { Host.portable(profile.path).use { host ->
            host.setRelayPreference(RelayPreference.Disabled)
            DocumentManager(context, host).use { manager ->
                val session = manager.create("PUBLIC whole encrypted copy", null, newOperation(), password)
                block(host, manager, session, manager.catalog().single())
            }
        } } finally { assertTrue(profile.deleteRecursively()) }
    }
    private class CountingOutput(val delegate: SelectedOutput, private val loseFinish: Boolean = false,
        private val entered: CountDownLatch? = null, private val release: CountDownLatch? = null) : SelectedOutput {
        val writes = AtomicInteger()
        val finishes = AtomicInteger()
        val closes = AtomicInteger()
        override fun write(offset: ULong, bytes: ByteArray): UInt {
            assertTrue(bytes.size <= 65536)
            writes.incrementAndGet()
            val written = delegate.write(offset, bytes)
            if (entered != null) { entered.countDown(); check(release!!.await(15, TimeUnit.SECONDS)) }
            return written
        }
        override fun finish() {
            finishes.incrementAndGet()
            delegate.finish()
            if (loseFinish) throw AndroidException.CommitUncertain()
        }
        override fun close() { closes.incrementAndGet(); delegate.close(); release?.countDown() }
    }

    @Test fun exactCapturedArchiveExportsAfterNewSaveAndBackgroundWithHistoryAndBlobsIntact() = fixture { host, manager, session, copy ->
        val input = destination("bin")
        val output = destination()
        val extracted = destination("bin")
        val importedProfile = File(context.noBackupFilesDir, "PUBLIC-copy-import-${UUID.randomUUID()}")
        try {
            val public = ByteArray(131073) { (it % 251).toByte() }
            input.writeBytes(public)
            beginPublicEntry(session)
            val editor = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC captured entry"))
            val imported = session.importAttachment(editor.form.draft, "PUBLIC captured.bin", null, newOperation(), SelectedDescriptors.openInput(context, uri(input)))
            assertEquals(SaveState.SAVED, session.save(imported.form.draft, imported.form.revision, newOperation()).state)
            val entry = session.overview("").entries.single().id
            val capturedBytes = File(copy.path).readBytes()
            val captured = host.captureDatabaseCopy(copy)
            try {
                val beforeIdentity = captured.identity()
                val changed = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC subsequent saved title"))
                assertEquals(SaveState.SAVED, session.save(changed.form.draft, changed.form.revision, newOperation()).state)
                assertNotEquals(beforeIdentity.fingerprint, host.captureDatabaseCopy(copy).use { it.identity().fingerprint })
                session.patchEntry(EntryTextField.USERNAME, TextEdit.Set("PUBLIC unfinished local edit"))
                session.persistForms()
                val sourceProcess = manager.process(session)
                manager.background()
                assertEquals(SessionAccess.REVOKED, session.access())
                manager.close(session)
                assertTrue(sourceProcess.exited())
                val sourceBytes = File(copy.path).readBytes()
                val draftFile = File(copy.path + "." + beforeIdentity.workingCopy + ".draft")
                val draftBytes = draftFile.readBytes()
                val catalog = host.workingCopies()
                output.writeBytes(ByteArray(capturedBytes.size + 19) { 42 })
                val selected = CountingOutput(SelectedDescriptors.openOutput(context, uri(output)))
                captured.export(selected)
                assertTrue(selected.writes.get() >= 3)
                assertEquals(1, selected.finishes.get())
                assertTrue(selected.closes.get() >= 1)
                assertArrayEquals(capturedBytes, output.readBytes())
                assertArrayEquals(sourceBytes, File(copy.path).readBytes())
                assertArrayEquals(draftBytes, draftFile.readBytes())
                assertEquals(catalog, host.workingCopies())
                assertEquals(beforeIdentity, captured.identity())
                assertEquals(copy.database, inspectImport(output.path))
                Host.portable(importedProfile.path).use { importedHost ->
                    importedHost.setRelayPreference(RelayPreference.Disabled)
                    DocumentManager(context, importedHost).use { importedManager ->
                        val reopened = importedManager.import(uri(output), password)
                        assertEquals("PUBLIC captured entry", reopened.entry(entry).title)
                        assertEquals(1, reopened.history(FormKind.ENTRY, entry).size)
                        assertTrue(reopened.forms().isEmpty())
                        val attachment = reopened.entryAttachments(entry, null).single()
                        assertEquals("PUBLIC captured.bin", attachment.name)
                        reopened.exportAttachment(entry, null, attachment.contents.single().blob,
                            SelectedDescriptors.openOutput(context, uri(extracted)))
                        assertArrayEquals(public, extracted.readBytes())
                    }
                }
            } finally { captured.cancel(); captured.close() }
        } finally { input.delete(); output.delete(); extracted.delete(); assertTrue(importedProfile.deleteRecursively()) }
    }

    @Test fun realFsyncFailureAndLostFinalAcknowledgementNeverSucceedAndRetryUsesTheSameCapture() = fixture { host, _, _, copy ->
        val destination = destination()
        val retry = destination()
        try { host.captureDatabaseCopy(copy).use { captured ->
            val original = File(copy.path).readBytes()
            val descriptor = ParcelFileDescriptor.open(File("/dev/null"), ParcelFileDescriptor.MODE_READ_WRITE)
            val failed = CountingOutput(SelectedDescriptorOutput(descriptor))
            assertThrows(AndroidException.StorageIo::class.java) { captured.export(failed) }
            assertFalse(descriptor.fileDescriptor.valid())
            assertEquals(1, failed.finishes.get())
            assertTrue(failed.closes.get() >= 1)
            val uncertain = CountingOutput(SelectedDescriptors.openOutput(context, uri(destination)), loseFinish = true)
            assertThrows(AndroidException.CommitUncertain::class.java) { captured.export(uncertain) }
            assertEquals(1, uncertain.finishes.get())
            assertTrue(uncertain.closes.get() >= 1)
            assertArrayEquals(original, destination.readBytes()) // Unknown confirmation never promises rollback.
            captured.export(SelectedDescriptors.openOutput(context, uri(retry)))
            assertArrayEquals(original, retry.readBytes())
            assertArrayEquals(original, File(copy.path).readBytes())
        } } finally { destination.delete(); retry.delete() }
    }

    @Test fun cancellingBusyNativeCopyClosesTheSelectedProviderAndCannotFinishOrRevive() = fixture { host, _, _, copy ->
        val destination = destination()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val executor = Executors.newSingleThreadExecutor()
        try { host.captureDatabaseCopy(copy).use { captured ->
            val source = File(copy.path).readBytes()
            val selected = CountingOutput(SelectedDescriptors.openOutput(context, uri(destination)), entered = entered, release = release)
            val pending = executor.submit { captured.export(selected) }
            try {
                assertTrue(entered.await(10, TimeUnit.SECONDS))
                captured.cancel()
                val failure = assertThrows(java.util.concurrent.ExecutionException::class.java) { pending.get(5, TimeUnit.SECONDS) }
                assertTrue(failure.cause is AndroidException.ExportCancelled)
                assertEquals(0, selected.finishes.get())
                assertTrue(selected.closes.get() >= 1)
                assertThrows(AndroidException.InvalidOptions::class.java) { selected.delegate.write(0uL, byteArrayOf(42)) }
                val late = CountingOutput(SelectedDescriptors.openOutput(context, uri(destination)))
                assertThrows(AndroidException.ExportCancelled::class.java) { captured.export(late) }
                assertEquals(0, late.writes.get())
                assertEquals(0, late.finishes.get())
                assertArrayEquals(source, File(copy.path).readBytes())
            } finally {
                release.countDown(); captured.cancel()
                try { pending.get(5, TimeUnit.SECONDS) }
                catch (error: java.util.concurrent.ExecutionException) { if (error.cause !is AndroidException.ExportCancelled) throw error }
            }
        } } finally { release.countDown(); executor.shutdownNow(); destination.delete() }
    }

    /** Bounded waits inspect content-free presentation, never logs or file contents. */
    private fun await(predicate: () -> Boolean) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(20)
        var accepted = false
        while (System.nanoTime() < deadline) {
            instrumentation.runOnMainSync { accepted = predicate() }
            if (accepted) return
            Thread.sleep(10)
        }
        fail("PUBLIC ciphertext-copy state did not reach the required phase")
    }
    @Test fun stalePickerResultCannotOpenOutputAndAConfirmedReplyAfterCopySwitchCannotPublishUiSuccess() = fixture { host, _, _, copy ->
        val destination = destination()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        var selectedSource: WorkingCopyView? = copy
        val openings = AtomicInteger()
        val output = CountingOutput(SelectedDescriptors.openOutput(context, uri(destination)), entered = entered, release = release)
        val state = CiphertextCopyState(scope, { host }, { selectedSource }, { openings.incrementAndGet(); output })
        try {
            instrumentation.runOnMainSync { state.begin(copy, "PUBLIC.taypeer") }
            await { state.phase == CiphertextCopyPhase.READY }
            var first: CiphertextCopyPicker? = null
            instrumentation.runOnMainSync { first = state.preparePicker(); state.pickerFailed(first!!.request) }
            var second: CiphertextCopyPicker? = null
            instrumentation.runOnMainSync {
                second = state.preparePicker()
                assertNotEquals(first!!.request, second!!.request)
                state.selected(first!!.request, uri(destination))
                assertEquals(CiphertextCopyPhase.SELECTING, state.phase)
            }
            assertEquals(0, openings.get())
            instrumentation.runOnMainSync { state.selected(second!!.request, uri(destination)) }
            assertTrue(entered.await(10, TimeUnit.SECONDS))
            instrumentation.runOnMainSync { selectedSource = null }
            release.countDown()
            await { !state.showing }
            assertEquals(1, output.finishes.get()) // Actual output can finish; stale presentation still cannot ACK.
            instrumentation.runOnMainSync { assertNull(state.operation); assertNull(state.identity) }
        } finally {
            release.countDown()
            instrumentation.runOnMainSync { state.cancel(); scope.cancel() }
            destination.delete()
        }
    }

    @Test fun uncertainPresentationRequiresExplicitNewOutputAndKeepsTheOriginalCaptureAfterNewSave() = fixture { host, _, session, copy ->
        val first = destination()
        val second = destination()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val openings = AtomicInteger()
        val state = CiphertextCopyState(scope, { host }, { copy }, { selected ->
            CountingOutput(SelectedDescriptors.openOutput(context, selected), loseFinish = openings.incrementAndGet() == 1)
        })
        try {
            val original = File(copy.path).readBytes()
            instrumentation.runOnMainSync { state.begin(copy, "PUBLIC.taypeer") }
            await { state.phase == CiphertextCopyPhase.READY }
            var operation = ""
            var fingerprint = ""
            instrumentation.runOnMainSync {
                operation = state.operation!!; fingerprint = state.identity!!.fingerprint
                state.selected(state.preparePicker()!!.request, uri(first))
            }
            await { state.phase == CiphertextCopyPhase.FAILED }
            instrumentation.runOnMainSync {
                assertEquals(CiphertextCopyFailure.UNCONFIRMED, state.failure)
                assertEquals(operation, state.operation)
                assertEquals(fingerprint, state.identity!!.fingerprint)
                assertTrue(state.canChoose)
            }
            assertArrayEquals(original, first.readBytes())
            beginPublicEntry(session)
            val changed = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC saved after unknown output"))
            assertEquals(SaveState.SAVED, session.save(changed.form.draft, changed.form.revision, newOperation()).state)
            assertFalse(original.contentEquals(File(copy.path).readBytes()))
            val current = File(copy.path).readBytes()
            instrumentation.runOnMainSync { state.selected(state.preparePicker()!!.request, uri(second)) }
            await { state.phase == CiphertextCopyPhase.SAVED }
            instrumentation.runOnMainSync {
                assertEquals(operation, state.operation)
                assertEquals(fingerprint, state.identity!!.fingerprint)
                assertNull(state.failure)
            }
            assertEquals(2, openings.get())
            assertArrayEquals(original, second.readBytes())
            assertArrayEquals(current, File(copy.path).readBytes())
        } finally {
            instrumentation.runOnMainSync { state.cancel(); scope.cancel() }
            first.delete(); second.delete()
        }
    }
}
