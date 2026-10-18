package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.DocumentProcess
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.BoundDocumentProcess
import dev.taypeer.platform.CiphertextDescriptors
import dev.taypeer.platform.DocumentManager
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

/** Actual worker Lock replies stay readable after observed Binder death and unbinding. */
@RunWith(AndroidJUnit4::class)
class DocumentShutdownTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC Android shutdown master"

    /** Delay the read, never synthesize an acknowledgement or a process exit. */
    private class HeldLockRead(private val process: BoundDocumentProcess) : DocumentProcess by process {
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        private val locking = AtomicBoolean()
        private val held = AtomicBoolean()
        private val lock = "\"Lock\"".toByteArray(Charsets.UTF_8)

        override fun write(bytes: ByteArray): UInt {
            // The unit command payload is separate from its length prefix. No other
            // request contents are retained or inspected by this fixture.
            if (bytes.contentEquals(lock)) locking.set(true)
            return process.write(bytes)
        }
        override fun read(count: UInt): ByteArray {
            if (locking.get() && held.compareAndSet(false, true)) {
                entered.countDown()
                check(release.await(10, TimeUnit.SECONDS))
            }
            return process.read(count)
        }
    }

    private inline fun fixture(block: (Host, DocumentSession, BoundDocumentProcess, HeldLockRead, CountDownLatch) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-shutdown-${UUID.randomUUID()}")
        val files = CiphertextDescriptors { anonymousCiphertext(context) }
        val process = BoundDocumentProcess(context, files, allocate = { anonymousCiphertext(context) })
        val held = HeldLockRead(process)
        val stopped = CountDownLatch(1)
        process.afterControlStop = { stopped.countDown() }
        var session: DocumentSession? = null
        try {
            Host.portable(profile.path).use { host ->
                try {
                    session = host.createDocument("PUBLIC shutdown", null, "PUBLIC shutdown create", password, held, files)
                    block(host, session!!, process, held, stopped)
                } finally {
                    held.release.countDown()
                    try { session?.closeSession() }
                    finally { process.close(); files.close(); session?.close() }
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    private fun closeAfterObservedExit(session: DocumentSession, process: BoundDocumentProcess, held: HeldLockRead, stopped: CountDownLatch) {
        val commands = Executors.newSingleThreadExecutor()
        val closed = commands.submit { session.closeSession() }
        try {
            assertTrue("PUBLIC Lock read entered", held.entered.await(5, TimeUnit.SECONDS))
            assertTrue("PUBLIC Binder exit cleanup completed", stopped.await(2, TimeUnit.SECONDS))
            assertTrue(process.exited())
            assertTrue(process.exitObservedNanos > 0)
            assertFalse("PUBLIC close still requires the real acknowledgement", closed.isDone)
            held.release.countDown()
            closed.get(3, TimeUnit.SECONDS) // Any native close failure remains a test failure.
            assertThrows(AndroidException::class.java) { session.overview("") }
        } finally {
            held.release.countDown()
            commands.shutdownNow()
        }
    }

    @Test fun cleanWorkerAcknowledgementDrainsAfterBinderDeathWithoutInventingDraftSuccess() = fixture { _, session, process, held, stopped ->
        assertTrue(session.forms().isEmpty())
        closeAfterObservedExit(session, process, held, stopped)
    }

    @Test fun delayedAcknowledgementConfirmsActualEncryptedDraftAndReopensItsExactIdentity() = fixture { host, session, process, held, stopped ->
        val initial = session.beginMetadata(FormKind.NEW_GROUP, null, null)
        val entered = session.patchMetadata(initial.form.draft, FormKind.NEW_GROUP, TextEdit.Keep,
            TextEdit.Set("PUBLIC incomplete group preserved before acknowledgement"))
        val path = host.workingCopies().single().path
        val before = File(path).readBytes()
        assertTrue(entered.form.dirty)
        assertTrue(session.overview("").groups.isEmpty())

        closeAfterObservedExit(session, process, held, stopped)
        assertArrayEquals(before, File(path).readBytes())
        process.close() // Release Binder writer capabilities before a new generation opens.
        DocumentManager(context, host).use { manager ->
            val reopened = manager.open(path, password)
            assertTrue(reopened.overview("").groups.isEmpty())
            val form = reopened.forms().single()
            assertEquals(entered.form.draft, form.draft)
            assertEquals(entered.form.revision, form.revision)
            assertEquals(entered.form.target, form.target)
            assertEquals(entered.form.kind, form.kind)
            assertEquals(entered.form, reopened.activeForm())
            // The real reopen path resumes this exact form. BeginCreateGroup
            // intentionally reserves an independent object and a new DraftId.
            reopened.resumeForm(form.draft)
            val restored = reopened.metadata(form.draft)
            assertEquals(entered.form.draft, restored.form.draft)
            assertEquals(entered.form.revision, restored.form.revision)
            assertEquals(entered.form.target, restored.form.target)
            assertEquals(entered.form.kind, restored.form.kind)
            assertEquals("", restored.name)
            assertEquals(entered.description, restored.description)
            assertTrue(restored.form.dirty)
            assertEquals(entered.form, reopened.forms().single())
            assertArrayEquals(before, File(path).readBytes())
        }
    }
}
