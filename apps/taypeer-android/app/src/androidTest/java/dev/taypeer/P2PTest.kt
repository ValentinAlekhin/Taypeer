package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.EnrollmentProgress
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.InvitationPhase
import dev.taypeer.bridge.RelayPreference
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
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
    private inner class Fixture(val localRelay: Boolean) {
        val directory = File(context.noBackupFilesDir, "PUBLIC-p2p-${UUID.randomUUID()}")
        val replicas = mutableListOf<Replica>()
        private val arguments = InstrumentationRegistry.getArguments()
        private val relayUrl by lazy { arguments.getString("relayUrl") ?: error("PUBLIC local TLS relay fixture URL required") }
        private val relayCertificate by lazy {
            android.util.Base64.decode(arguments.getString("relayCertificate") ?: error("PUBLIC local TLS relay fixture certificate required"), android.util.Base64.DEFAULT)
        }
        fun start(replica: Replica) {
            if (localRelay) replica.host.startPublicTestRelay(relayUrl, relayCertificate)
            else replica.host.startExchange()
        }
        fun add(): Replica {
            val host = Host.portable(File(directory, "replica-${replicas.size}").path)
            host.setRelayPreference(RelayPreference.Disabled)
            return Replica(host, DocumentManager(context, host)).also(replicas::add)
        }
        fun restart(index: Int): Replica {
            val previous = replicas[index]
            previous.host.stopExchange()
            try { previous.documents.close() } finally { previous.host.close() }
            val host = Host.portable(File(directory, "replica-$index").path)
            return Replica(host, DocumentManager(context, host)).also { replicas[index] = it }
        }
        fun wake() = replicas.forEach { it.host.wakeExchange() }
        fun close() {
            // No delivery may race an assertion that the fixture workers closed orderly.
            replicas.forEach { it.host.stopExchange() }
            replicas.asReversed().forEach {
                try { it.documents.close() } finally { it.host.close() }
            }
            assertTrue(directory.deleteRecursively())
        }
    }
    private fun fixture(count: Int, localRelay: Boolean = false, scenario: (Fixture) -> Unit) {
        val fixture = Fixture(localRelay)
        assertTrue(fixture.directory.mkdir())
        try { repeat(count) { fixture.add() }; scenario(fixture) }
        finally { fixture.close() }
    }
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
        val request = pendingRequest(manager, session, recipient, database)
        assertEquals(request, recipient.documents.pendingJoins().single().request)
        session.approveInvitation(request)
        return receive(recipient, request, database)
    }
    private fun pendingRequest(manager: Replica, session: DocumentSession, recipient: Replica, database: String): String {
        val progress = recipient.documents.join(session.createInvitation(), password)
        assertTrue("PUBLIC enrollment pending", progress is EnrollmentProgress.Pending)
        val request = (progress as EnrollmentProgress.Pending).request
        assertTrue(recipient.documents.catalog().isEmpty())
        until("PUBLIC manager sees proof") {
            manager.host.wakeExchange()
            manager.host.exchangeView(database).invitations.any { it.request == request && it.phase == InvitationPhase.REQUESTED }
        }
        return request
    }
    private fun receive(recipient: Replica, request: String, database: String): File {
        until("PUBLIC recipient receives approved ciphertext") {
            when (val received = recipient.documents.resumeJoin(request, password)) {
                is EnrollmentProgress.Received -> { assertEquals(database, received.database); true }
                is EnrollmentProgress.Pending -> false
                is EnrollmentProgress.Rejected -> throw AssertionError("PUBLIC enrollment rejected")
            }
        }
        assertTrue(recipient.documents.pendingJoins().isEmpty())
        return File(recipient.documents.catalog().single().path)
    }
    @Test fun pendingEnrollmentSurvivesRecipientHostRestartWithoutTheBearerCode() = fixture(2) { fixture ->
        val manager = fixture.replicas[0]
        val recipient = fixture.replicas[1]
        val authority = manager.documents.create("PUBLIC resumable enrollment", null, "PUBLIC resumable create", password)
        beginPublicEntry(authority)
        save(authority, EntryTextField.TITLE, "PUBLIC durable invitation payload", "PUBLIC resumable snapshot")
        val database = authority.databaseId()
        manager.host.startExchange()
        val request = pendingRequest(manager, authority, recipient, database)
        val restarted = fixture.restart(1)
        assertEquals(request, restarted.documents.pendingJoins().single().request)
        assertTrue(restarted.documents.catalog().isEmpty())
        restarted.host.startExchange()
        assertTrue(restarted.documents.resumeJoin(request, password) is EnrollmentProgress.Pending)
        authority.approveInvitation(request)
        val received = receive(restarted, request, database)
        // The common catalog is published while the database remains locked.
        assertEquals(database, restarted.documents.catalog().single().database)
        val reopened = restarted.documents.open(received.path, password)
        assertEquals("PUBLIC durable invitation payload", reopened.overview("").entries.single().title)
        val thirdLifetime = fixture.restart(1)
        assertTrue(thirdLifetime.documents.pendingJoins().isEmpty())
        assertEquals(received.path, thirdLifetime.documents.catalog().single().path)
        assertEquals("PUBLIC durable invitation payload", thirdLifetime.documents.open(received.path, password).overview("").entries.single().title)
    }
    @Test fun rejectionAndCancellationNeverPublishAWorkingCopyOrMembership() = fixture(3) { fixture ->
        val manager = fixture.replicas[0]
        val authority = manager.documents.create("PUBLIC denied enrollment", null, "PUBLIC denied create", password)
        val database = authority.databaseId()
        manager.host.startExchange()
        listOf(true, false).forEachIndexed { index, reject ->
            val recipient = fixture.replicas[index + 1]
            val request = pendingRequest(manager, authority, recipient, database)
            authority.closeInvitation(request, reject)
            assertTrue(recipient.documents.resumeJoin(request, password) is EnrollmentProgress.Rejected)
            assertTrue(recipient.documents.catalog().isEmpty())
            val phase = if (reject) InvitationPhase.REJECTED else InvitationPhase.CANCELLED
            assertEquals(phase, manager.host.exchangeView(database).invitations.single { it.request == request }.phase)
            val restarted = fixture.restart(index + 1)
            restarted.host.startExchange()
            assertTrue(restarted.documents.resumeJoin(request, password) is EnrollmentProgress.Rejected)
            assertTrue(restarted.documents.catalog().isEmpty())
            assertEquals(1, manager.host.exchangeView(database).devices.size)
        }
    }
    @Test fun offlineConcurrentEditsConvergeAfterHostRestartWithoutDuplicatingHistory() = fixture(2, localRelay = true) { fixture ->
        val manager = fixture.replicas[0]
        val recipient = fixture.replicas[1]
        val authority = manager.documents.create("PUBLIC offline convergence", null, "PUBLIC offline create", password)
        beginPublicEntry(authority)
        save(authority, EntryTextField.TITLE, "PUBLIC initial offline entry", "PUBLIC offline initial snapshot")
        val database = authority.databaseId()
        val entry = authority.overview("").entries.single().id
        fixture.start(manager)
        fixture.start(recipient)
        val recipientFile = enroll(manager, authority, recipient, database)
        val peer = recipient.documents.open(recipientFile.path, password)
        fixture.replicas.forEach { it.host.stopExchange() }
        authority.beginEditEntry(entry)
        peer.beginEditEntry(entry)
        save(authority, EntryTextField.TITLE, "PUBLIC concurrent manager title", "PUBLIC offline manager snapshot")
        save(peer, EntryTextField.TITLE, "PUBLIC concurrent recipient title", "PUBLIC offline recipient snapshot")
        assertEquals("PUBLIC concurrent manager title", authority.entry(entry).title)
        assertEquals("PUBLIC concurrent recipient title", peer.entry(entry).title)
        val restarted = fixture.restart(1)
        val reopened = restarted.documents.open(recipientFile.path, password)
        assertEquals("PUBLIC concurrent recipient title", reopened.entry(entry).title)
        fixture.replicas.forEach(fixture::start)
        val expectedNames = setOf("PUBLIC initial offline entry", "PUBLIC concurrent manager title", "PUBLIC concurrent recipient title")
        until("PUBLIC original concurrent revisions converge") {
            fixture.wake()
            listOf(authority, reopened).all { it.history(FormKind.ENTRY, entry).map { row -> row.name }.toSet() == expectedNames }
                && authority.entry(entry).title == reopened.entry(entry).title
        }
        val confirmed = authority.history(FormKind.ENTRY, entry).map { it.id }.toSet()
        assertEquals(3, confirmed.size)
        assertEquals(confirmed, reopened.history(FormKind.ENTRY, entry).map { it.id }.toSet())
        // Redelivery plus another host restart must retain the original three versions.
        fixture.replicas.forEach { it.host.stopExchange() }
        val lastLifetime = fixture.restart(1)
        val lastSession = lastLifetime.documents.open(recipientFile.path, password)
        fixture.replicas.forEach(fixture::start)
        until("PUBLIC repeated exchange completes") {
            fixture.wake()
            fixture.replicas.all { replica -> replica.host.exchangeView(database).devices.any { !it.local && it.lastExchangeSucceeded == true } }
        }
        assertEquals(confirmed, lastSession.history(FormKind.ENTRY, entry).map { it.id }.toSet())
        assertEquals(confirmed, authority.history(FormKind.ENTRY, entry).map { it.id }.toSet())
        assertEquals(1, lastSession.overview("").entries.size)
        assertEquals(authority.entry(entry).title, lastSession.entry(entry).title)
    }
    @Test fun threeReplicasReceiveWhileLockedAndApplyOnlyAfterReopening() {
        val directory = File(context.noBackupFilesDir, "PUBLIC-p2p-${UUID.randomUUID()}")
        assertTrue(directory.mkdir())
        val replicas = mutableListOf<Replica>()
        try {
            repeat(3) { index ->
                val host = Host.portable(File(directory, "replica-$index").path)
                host.setRelayPreference(RelayPreference.Disabled)
                replicas += Replica(host, DocumentManager(context, host))
            }
            val (first, second, third) = replicas
            val authority = first.documents.create("PUBLIC exchange", null, "PUBLIC p2p create", password)
            beginPublicEntry(authority)
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
            reopenedSecond.beginEditEntry(entry)
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
