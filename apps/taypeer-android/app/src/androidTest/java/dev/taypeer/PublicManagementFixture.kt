package dev.taypeer

import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import org.junit.Assert.*
import java.io.File
import java.util.UUID
import java.util.concurrent.TimeUnit

/** Two real profiles, Binder capabilities, isolated workers and authenticated direct Iroh endpoints. */
internal class PublicManagementFixture : AutoCloseable {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private data class Replica(val host: Host, val documents: DocumentManager)
    private val directory = File(context.noBackupFilesDir, "PUBLIC-management-${UUID.randomUUID()}")
    private val replicas = mutableListOf<Replica>()
    val password = "PUBLIC management unchanged master password"
    val savedPassword = "PUBLIC management saved secret"
    val name = "PUBLIC management database"
    lateinit var sourceSession: DocumentSession; private set
    var peerSession: DocumentSession? = null; private set
    lateinit var sourceCopy: WorkingCopyView; private set
    lateinit var peerCopy: WorkingCopyView; private set
    lateinit var database: String; private set
    lateinit var entry: String; private set
    val sourceHost get() = replicas[0].host
    val peerHost get() = replicas[1].host
    val sourceDocuments get() = replicas[0].documents
    val peerDocuments get() = replicas[1].documents
    val recipient get() = peerHost.exchangeView(database).devices.single { it.local }.id

    init {
        assertTrue(directory.mkdir())
        repeat(2) { replicas += replica(it) }
    }
    private fun replica(index: Int): Replica {
        val host = Host.portable(File(directory, "replica-$index").path)
        host.setRelayPreference(RelayPreference.Disabled)
        return Replica(host, DocumentManager(context, host))
    }
    fun prepare(openPeer: Boolean = true) {
        sourceSession = sourceDocuments.create(name, null, newOperation(), password)
        sourceSession.beginCreateEntry(publicGroup(sourceSession))
        sourceSession.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC management saved entry"))
        sourceSession.patchEntry(EntryTextField.PASSWORD, TextEdit.Set(savedPassword))
        var editor = sourceSession.editor()
        entry = sourceSession.save(editor.form.draft, editor.form.revision, newOperation()).form.target!!
        sourceSession.deleteForm(editor.form.draft)
        sourceSession.beginEditEntry(entry)
        sourceSession.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC management current entry"))
        editor = sourceSession.editor()
        sourceSession.save(editor.form.draft, editor.form.revision, newOperation())
        sourceSession.deleteForm(editor.form.draft)
        database = sourceSession.databaseId()
        sourceCopy = sourceDocuments.catalog().single()
        replicas.forEach { it.host.startExchange() }
        val progress = peerDocuments.join(sourceSession.createInvitation(), password)
        assertTrue(progress is EnrollmentProgress.Pending)
        val request = (progress as EnrollmentProgress.Pending).request
        await("PUBLIC manager receives enrollment proof") {
            sourceHost.wakeExchange()
            sourceHost.exchangeView(database).invitations.any { it.request == request && it.phase == InvitationPhase.REQUESTED }
        }
        sourceSession.approveInvitation(request)
        await("PUBLIC recipient receives encrypted working copy") {
            when (val state = peerDocuments.resumeJoin(request, password)) {
                is EnrollmentProgress.Received -> { assertEquals(database, state.database); true }
                is EnrollmentProgress.Pending -> false
                is EnrollmentProgress.Rejected -> throw AssertionError("PUBLIC management fixture enrollment rejected")
            }
        }
        peerCopy = peerDocuments.catalog().single()
        await("PUBLIC two admitted identities") {
            replicas.forEach { it.host.wakeExchange() }
            replicas.all { it.host.exchangeView(database).devices.size == 2 }
        }
        if (openPeer) reopenPeer()
    }
    fun reopenPeer(): DocumentSession {
        closePeer()
        return peerDocuments.open(peerCopy.path, password).also { peerSession = it }
    }
    fun closePeer() {
        peerSession?.let { it.lock(); peerDocuments.close(it) }
        peerSession = null
    }
    fun restartPeer(open: Boolean = true) {
        peerSession = null
        val previous = replicas[1]
        previous.host.stopExchange()
        try { previous.documents.close() } finally { previous.host.close() }
        replicas[1] = replica(1)
        peerHost.startExchange()
        if (open) peerSession = peerDocuments.open(peerCopy.path, password)
    }
    fun propose(operation: String = newOperation()): ManagementHandoffView = afterActualApplication(sourceSession) {
        sourceSession.proposeManagement(recipient, operation)
    }
    fun accept(intent: ManagementIntentView): ManagementHandoffView = afterActualApplication(peerSession!!) {
        peerSession!!.acceptManagement(intent)
    }
    /** Only a typed pending-basis refusal is retried after a new actual application observation. */
    private fun afterActualApplication(session: DocumentSession, action: () -> ManagementHandoffView): ManagementHandoffView {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        while (true) {
            val beforeSource = sourceSession.applicationState()?.revision
            val beforePeer = peerSession?.applicationState()?.revision
            try { return action() }
            catch (error: AndroidException.Management) {
                assertEquals(ManagementFailure.PENDING_APPLY, error.failure)
                await("PUBLIC actual automatic application advances", deadline) {
                    sourceSession.applicationState()?.let { it.revision != beforeSource && it.status != ApplicationPhase.FAILED } == true ||
                        peerSession?.applicationState()?.let { it.revision != beforePeer && it.status != ApplicationPhase.FAILED } == true
                }
                assertTrue(session.access() == SessionAccess.OPEN)
                assertTrue(System.nanoTime() < deadline)
            }
        }
    }
    fun complete(intent: ManagementIntentView): ManagementHandoffView {
        var result: ManagementHandoffView? = null
        await("PUBLIC both sides confirm actual applied management") {
            val row = sourceHost.retryManagement(intent)
            if (row.phase != ManagementPhase.COMPLETED) peerHost.retryManagement(intent)
            result = row
            row.phase == ManagementPhase.COMPLETED && row.applied != null
        }
        return result!!
    }
    fun stopNetwork() = replicas.forEach { it.host.stopExchange() }
    override fun close() {
        replicas.forEach { it.host.stopExchange() }
        replicas.asReversed().forEach { try { it.documents.close() } finally { it.host.close() } }
        assertTrue(directory.deleteRecursively())
    }
    companion object {
        fun await(stage: String, deadline: Long = System.nanoTime() + TimeUnit.SECONDS.toNanos(30), condition: () -> Boolean) {
            while (!condition()) {
                assertTrue(stage, System.nanoTime() < deadline)
                Thread.sleep(25)
            }
        }
    }
}
