package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.EnrollmentProgress
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.InvitationPhase
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.TimeUnit

/** Three real hosts and isolated document workers exchange signed ciphertext over native Iroh. */
@RunWith(AndroidJUnit4::class)
class P2PTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC Android P2P master"
    private data class Replica(val host: Host, val documents: DocumentManager)
    private fun until(stage: String, predicate: () -> Boolean) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        var observed = predicate()
        while (!observed && System.nanoTime() < deadline) { Thread.sleep(50); observed = predicate() }
        assertTrue(stage, observed) // The stage is fixed public text, never a code or document.
    }
    private fun save(session: DocumentSession, field: EntryTextField, value: String, operation: String) {
        val form = session.patchEntry(field, TextEdit.Set(value)).form
        assertEquals(SaveState.SAVED, session.save(form.draft, form.revision, operation).state)
    }
    private fun enroll(manager: Replica, session: DocumentSession, recipient: Replica, database: String): File {
        val code = session.createInvitation()
        val progress = recipient.documents.join(code)
        assertTrue("PUBLIC enrollment pending", progress is EnrollmentProgress.Pending)
        val request = (progress as EnrollmentProgress.Pending).request
        assertTrue(recipient.documents.catalog().isEmpty())
        assertEquals(request, recipient.documents.pendingJoins().single().request)
        until("PUBLIC manager sees proof") {
            manager.host.wakeExchange()
            manager.host.exchangeView(database).invitations.any { it.request == request && it.phase == InvitationPhase.REQUESTED }
        }
        session.approveInvitation(request)
        until("PUBLIC recipient receives approved ciphertext") {
            when (val received = recipient.documents.resumeJoin(request)) {
                is EnrollmentProgress.Received -> { assertEquals(database, received.database); true }
                is EnrollmentProgress.Pending -> false
                is EnrollmentProgress.Rejected -> throw AssertionError("PUBLIC enrollment rejected")
            }
        }
        assertTrue(recipient.documents.pendingJoins().isEmpty())
        return File(recipient.documents.catalog().single().path)
    }
    @Test fun threeReplicasReceiveWhileLockedAndApplyOnlyAfterReopening() {
        val directory = File(context.noBackupFilesDir, "PUBLIC-p2p-${UUID.randomUUID()}")
        assertTrue(directory.mkdir())
        val replicas = mutableListOf<Replica>()
        try {
            repeat(3) { index ->
                val host = Host(File(directory, "replica-$index").path, KeystoreCredentials(context))
                replicas += Replica(host, DocumentManager(context, host))
            }
            val (first, second, third) = replicas
            val authority = first.documents.create("PUBLIC exchange", null, "PUBLIC p2p create", password)
            authority.beginEntry(null, null)
            save(authority, EntryTextField.TITLE, "PUBLIC initial", "PUBLIC initial snapshot")
            val database = authority.databaseId()
            val entry = authority.overview("").entries.single().id
            replicas.forEach { it.host.startExchange() }
            val secondFile = enroll(first, authority, second, database)
            val thirdFile = enroll(first, authority, third, database)
            until("PUBLIC three verified members before editing") {
                replicas.forEach { it.host.wakeExchange() }
                replicas.all { it.host.exchangeView(database).devices.size == 3 }
            }
            val secondSession = second.documents.open(secondFile.path, password)
            val thirdSession = third.documents.open(thirdFile.path, password)
            assertEquals("PUBLIC initial", secondSession.entry(entry).title)
            assertEquals("PUBLIC initial", thirdSession.entry(entry).title)
            assertTrue(secondSession.overview("").writable)
            assertTrue(thirdSession.overview("").writable)
            second.documents.lock(secondSession)
            second.documents.close(secondSession)
            val secondBefore = secondFile.readBytes()
            save(authority, EntryTextField.TITLE, "PUBLIC received while locked", "PUBLIC locked snapshot")
            until("PUBLIC durable receive while locked") {
                first.host.wakeExchange(); second.host.wakeExchange()
                !secondBefore.contentEquals(secondFile.readBytes())
            }
            val reopenedSecond = second.documents.open(secondFile.path, password)
            assertEquals("PUBLIC received while locked", reopenedSecond.entry(entry).title)
            third.documents.background()
            assertThrows(AndroidException::class.java) { thirdSession.overview("") }
            third.documents.close(thirdSession)
            val thirdBefore = thirdFile.readBytes()
            save(authority, EntryTextField.TITLE, "PUBLIC received in background", "PUBLIC background snapshot")
            until("PUBLIC durable receive in background") {
                first.host.wakeExchange(); third.host.wakeExchange()
                !thirdBefore.contentEquals(thirdFile.readBytes())
            }
            val reopenedThird = third.documents.open(thirdFile.path, password)
            assertEquals("PUBLIC received in background", reopenedThird.entry(entry).title)
            reopenedSecond.beginEntry(entry, null)
            save(reopenedSecond, EntryTextField.USERNAME, "PUBLIC peer edit", "PUBLIC peer snapshot")
            until("PUBLIC three replicas converge") {
                replicas.forEach { it.host.wakeExchange() }
                listOf(authority, reopenedSecond, reopenedThird).all {
                    val row = it.entry(entry)
                    row.title == "PUBLIC received in background" && row.username == "PUBLIC peer edit"
                }
            }
            assertEquals(3, first.host.exchangeView(database).devices.size)
        } finally {
            // Stop fixture network delivery before asserting orderly document teardown.
            // Production background revocation during active delivery is tested above.
            replicas.forEach { it.host.stopExchange() }
            replicas.asReversed().forEach {
                try { it.documents.close() } finally { it.host.close() }
            }
            assertTrue(directory.deleteRecursively())
        }
    }
}
