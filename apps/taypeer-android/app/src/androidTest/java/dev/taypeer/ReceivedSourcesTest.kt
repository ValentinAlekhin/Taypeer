package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.ChangedObject
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

/** Real Iroh delivery and isolated workers retain late closed-generation changes for explicit handling. */
@RunWith(AndroidJUnit4::class)
class ReceivedSourcesTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC closed-source master"
    private data class Replica(val host: Host, val documents: DocumentManager, val profile: File)

    /** Runs actual worker commands and loses only one already durable extraction response. */
    private class LostExtractionReplyCommands(var session: DocumentSession) : SessionCommands {
        var dropped: ChangedObject? = null; private set
        private var dropNextExtraction = true
        override fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
            completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit, discarded: (T) -> Unit): Boolean {
            started()
            val result = try { command(session) } catch (failure: AndroidException) {
                if (failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions) rejected()
                else failed()
                return true
            }
            if (result is ChangedObject && dropNextExtraction) {
                dropNextExtraction = false
                dropped = result
                failed()
            } else completed(result)
            return true
        }
    }

    private fun fixture(scenario: (List<Replica>) -> Unit) {
        val directory = File(context.noBackupFilesDir, "PUBLIC-closed-sources-${UUID.randomUUID()}")
        val replicas = mutableListOf<Replica>()
        try {
            repeat(2) { index ->
                val profile = File(directory, "replica-$index")
                val host = Host.portable(profile.path)
                host.setRelayPreference(RelayPreference.Disabled)
                replicas += Replica(host, DocumentManager(context, host), profile)
            }
            scenario(replicas)
        } finally {
            replicas.forEach { it.host.stopExchange() }
            replicas.asReversed().forEach { replica ->
                try { replica.documents.close() } finally { replica.host.close() }
            }
            assertTrue(directory.deleteRecursively())
        }
    }

    private fun start(replica: Replica) {
        val arguments = InstrumentationRegistry.getArguments()
        val url = arguments.getString("relayUrl") ?: error("PUBLIC local TLS fixture URL required")
        val certificate = android.util.Base64.decode(
            arguments.getString("relayCertificate") ?: error("PUBLIC local TLS fixture CA required"),
            android.util.Base64.DEFAULT)
        replica.host.startPublicTestRelay(url, certificate)
    }

    /** Polls a bounded observable condition; it never retries arbitrary command failures. */
    private fun until(stage: String, predicate: () -> Boolean) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        var observed = predicate()
        while (!observed && System.nanoTime() < deadline) {
            Thread.sleep(50)
            observed = predicate()
        }
        assertTrue(stage, observed)
    }

    private fun enroll(authority: Replica, session: DocumentSession, recipient: Replica): String {
        val progress = recipient.documents.join(session.createInvitation(), password)
        assertTrue("PUBLIC enrollment pending", progress is EnrollmentProgress.Pending)
        val request = (progress as EnrollmentProgress.Pending).request
        until("PUBLIC original proof reaches managing replica") {
            authority.host.wakeExchange(); recipient.host.wakeExchange()
            authority.host.exchangeView(session.databaseId()).invitations.any {
                it.request == request && it.phase == InvitationPhase.REQUESTED
            }
        }
        session.approveInvitation(request)
        until("PUBLIC approved original ciphertext reaches recipient") {
            when (val received = recipient.documents.resumeJoin(request, password)) {
                is EnrollmentProgress.Received -> { assertEquals(session.databaseId(), received.database); true }
                is EnrollmentProgress.Pending -> false
                is EnrollmentProgress.Rejected -> throw AssertionError("PUBLIC enrollment rejected")
            }
        }
        return recipient.documents.catalog().single().path
    }

    private fun lateSave(session: DocumentSession, entry: String, title: String, secret: String) {
        session.beginEditEntry(entry)
        session.patchEntry(EntryTextField.TITLE, TextEdit.Set(title))
        val editor = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set(secret))
        assertEquals(SaveState.SAVED, session.save(editor.form.draft, editor.form.revision, newOperation()).state)
        session.deleteForm(editor.form.draft)
    }

    @Test fun lateConflictInspectionKeepsEveryPasswordAlternativeAndExtractionUsesSharedProjection() = fixture { replicas ->
        val (manager, recipient) = replicas
        val authority = manager.documents.create("PUBLIC conflicting source database", null, newOperation(), password)
        val group = publicGroup(authority)
        authority.beginCreateEntry(group)
        authority.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC conflicting original entry"))
        val initial = authority.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC original conflict password"))
        val originalReceipt = authority.save(initial.form.draft, initial.form.revision, newOperation())
        assertEquals(SaveState.SAVED, originalReceipt.state)
        val original = originalReceipt.form.target!!
        authority.deleteForm(originalReceipt.form.draft)
        val database = authority.databaseId()
        val path = manager.documents.catalog().single().path
        replicas.forEach(::start)
        val recipientPath = enroll(manager, authority, recipient)
        val peer = recipient.documents.open(recipientPath, password)
        until("PUBLIC conflict fixture replicas share enrollment") {
            replicas.forEach { it.host.wakeExchange() }
            replicas.all { it.host.exchangeView(database).devices.size == 2 }
        }
        replicas.forEach { it.host.stopExchange() }

        val expectedPasswords = setOf("PUBLIC branch password manager", "PUBLIC branch password peer")
        listOf(authority, peer).zip(expectedPasswords).forEach { (session, secret) ->
            session.beginEditEntry(original)
            val edited = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set(secret))
            assertEquals(SaveState.SAVED, session.save(edited.form.draft, edited.form.revision, newOperation()).state)
            session.deleteForm(edited.form.draft)
        }
        replicas.forEach(::start)
        until("PUBLIC concurrent password alternatives converge without overwriting either branch") {
            replicas.forEach { it.host.wakeExchange() }
            listOf(authority, peer).all { session ->
                session.entry(original).alternatives && session.history(FormKind.ENTRY, original).size == 3
            }
        }
        replicas.forEach { it.host.stopExchange() }
        assertEquals(authority.history(FormKind.ENTRY, original).map { it.id }.toSet(),
            peer.history(FormKind.ENTRY, original).map { it.id }.toSet())
        val selectedPassword = peer.reveal(original, null, null)
        assertTrue(expectedPasswords.contains(selectedPassword))

        authority.trash(FormKind.ENTRY, original, newOperation())
        val old = authority.trashRows().single()
        val purge = authority.prepareTrashPurge(old.kind, old.target, old.generation)
        authority.confirmTrashReview(purge.review, newOperation())
        authority.discardTrashReview(purge.review)
        peer.beginEditEntry(original)
        // Only title is addressed: both immutable password alternatives must remain retained.
        val late = peer.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC late source with password alternatives"))
        assertEquals(SaveState.SAVED, peer.save(late.form.draft, late.form.revision, newOperation()).state)
        peer.deleteForm(late.form.draft)
        replicas.forEach(::start)
        until("PUBLIC closed source with original password alternatives arrives") {
            replicas.forEach { it.host.wakeExchange() }
            authority.pendingSources().size == 1
        }
        replicas.forEach { it.host.stopExchange() }
        val source = authority.pendingSources().single()
        assertEquals(old.generation, source.generation)
        val beforeInspect = File(path).readBytes()
        val localState = replicas.flatMap { publicPortableStates(it.profile).entries }.associate { it.toPair() }
        val inspection = authority.inspectPending(source.source)
        assertEquals("PUBLIC late source with password alternatives", inspection.entry!!.title)
        assertTrue(inspection.entry!!.alternatives)
        assertEquals(2, inspection.passwordVariants.size)
        assertTrue(inspection.passwordVariants.all { it.origins.isNotEmpty() })
        val revealed = inspection.passwordVariants.associate { variant ->
            variant.origins to authority.revealPending(source.source, variant.origins)
        }
        assertEquals(expectedPasswords, revealed.values.toSet())
        assertPublicPortableStatesUnchanged(localState)
        assertArrayEquals(beforeInspect, File(path).readBytes())
        assertTrue(authority.overview("").entries.isEmpty())
        assertThrows(AndroidException::class.java) {
            authority.revealPending(source.source, inspection.passwordVariants.flatMap { it.origins })
        }
        assertArrayEquals(beforeInspect, File(path).readBytes())

        val operation = newOperation()
        val extracted = authority.recoverPending(source.source, group, operation)
        assertEquals(FormKind.ENTRY, extracted.kind)
        assertNotEquals(original, extracted.target)
        assertEquals(selectedPassword, authority.reveal(extracted.target, null, null))
        assertEquals(1, authority.history(FormKind.ENTRY, extracted.target).size)
        assertFalse(authority.entry(extracted.target).alternatives)
        assertTrue(authority.pendingSources().isEmpty())
        val confirmed = File(path).readBytes()
        assertEquals(extracted, authority.recoverPending(source.source, group, operation))
        assertArrayEquals(confirmed, File(path).readBytes())
        authority.lock()
        manager.documents.close(authority)
        val reopened = manager.documents.open(path, password)
        assertEquals(extracted, reopened.recoverPending(source.source, group, operation))
        assertEquals(1, reopened.history(FormKind.ENTRY, extracted.target).size)
        assertEquals(selectedPassword, reopened.reveal(extracted.target, null, null))
        assertNull(reopened.entryProperties(extracted.target).accessedAt)
        assertEquals(expectedPasswords, reopened.inspectPending(source.source).passwordVariants.map {
            reopened.revealPending(source.source, it.origins)
        }.toSet())
        assertTrue(reopened.pendingSources().isEmpty())
        assertThrows(AndroidException::class.java) { reopened.entry(original) }
    }

    @Test fun lateClosedSourcesRemainMaskedAndExtractionReplaysAcrossLockAndReopen() = fixture { replicas ->
        val (manager, recipient) = replicas
        val authority = manager.documents.create("PUBLIC closed source database", null, newOperation(), password)
        val group = publicGroup(authority)
        authority.beginCreateEntry(group)
        authority.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC original closed entry"))
        val ready = authority.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC original closed password"))
        val originalReceipt = authority.save(ready.form.draft, ready.form.revision, newOperation())
        assertEquals(SaveState.SAVED, originalReceipt.state)
        val original = originalReceipt.form.target!!
        authority.deleteForm(originalReceipt.form.draft)
        val database = authority.databaseId()
        val path = manager.documents.catalog().single().path
        replicas.forEach(::start)
        val recipientPath = enroll(manager, authority, recipient)
        val peer = recipient.documents.open(recipientPath, password)
        assertNotEquals(android.os.Process.myPid(), manager.documents.process(authority).pid)
        assertNotEquals(manager.documents.process(authority).pid, recipient.documents.process(peer).pid)
        until("PUBLIC two admitted replicas share original control state") {
            replicas.forEach { it.host.wakeExchange() }
            replicas.all { it.host.exchangeView(database).devices.size == 2 }
        }
        val members = manager.host.exchangeView(database).devices.map { it.id }.toSet()
        replicas.forEach { it.host.stopExchange() }

        authority.trash(FormKind.ENTRY, original, newOperation())
        val old = authority.trashRows().single()
        val purge = authority.prepareTrashPurge(old.kind, old.target, old.generation)
        val purgeOperation = newOperation()
        authority.confirmTrashReview(purge.review, purgeOperation)
        authority.confirmTrashReview(purge.review, purgeOperation)
        authority.discardTrashReview(purge.review)
        assertTrue(authority.overview("").entries.isEmpty())
        assertTrue(authority.trashRows().isEmpty())

        lateSave(peer, original, "PUBLIC late closed first", "PUBLIC late first password")
        lateSave(peer, original, "PUBLIC late closed second", "PUBLIC late second password")
        replicas.forEach(::start)
        until("PUBLIC original late events are retained outside active entries") {
            replicas.forEach { it.host.wakeExchange() }
            authority.pendingSources().size == 2
        }
        replicas.forEach { it.host.stopExchange() }
        assertTrue(authority.overview("").entries.isEmpty())
        assertTrue(authority.trashRows().isEmpty())
        assertThrows(AndroidException::class.java) { authority.entry(original) }
        assertThrows(AndroidException::class.java) { authority.reveal(original, null, null) }
        // These original changes are admitted history, so the unadmitted-hash API is distinct.
        assertTrue(authority.receivedSources().isEmpty())
        val sources = authority.pendingSources()
        assertEquals(2, sources.size)
        sources.forEach { source ->
            assertEquals(FormKind.ENTRY, source.kind)
            assertEquals(original, source.target)
            assertEquals(old.generation, source.generation)
            assertNotNull(source.revision)
        }
        val beforeInspect = File(path).readBytes()
        val localState = replicas.flatMap { publicPortableStates(it.profile).entries }.associate { it.toPair() }
        val inspections = sources.associateWith { authority.inspectPending(it.source) }
        val selected = inspections.entries.single { it.value.entry?.title == "PUBLIC late closed first" }
        val remaining = sources.single { it.source != selected.key.source }
        val inspected = selected.value
        assertNull(inspected.group)
        assertEquals(true, inspected.entry!!.hasPassword)
        val origins = inspected.passwordVariants.single().origins
        assertTrue(origins.isNotEmpty())
        assertThrows(AndroidException::class.java) {
            authority.revealPending(selected.key.source, listOf("PUBLIC wrong immutable origins"))
        }
        assertEquals("PUBLIC late first password", authority.revealPending(selected.key.source, origins))
        assertPublicPortableStatesUnchanged(localState)
        assertArrayEquals(beforeInspect, File(path).readBytes())
        assertTrue(authority.overview("").entries.isEmpty())

        val missingDestination = File(path).readBytes()
        assertThrows(AndroidException::class.java) {
            authority.recoverPending(selected.key.source, null, newOperation())
        }
        assertArrayEquals(missingDestination, File(path).readBytes())
        val operation = newOperation()
        val extracted = authority.recoverPending(selected.key.source, group, operation)
        assertEquals(FormKind.ENTRY, extracted.kind)
        assertNotEquals(original, extracted.target)
        assertEquals(group, authority.entry(extracted.target).group)
        assertEquals("PUBLIC late closed first", authority.entry(extracted.target).title)
        assertEquals("PUBLIC late first password", authority.reveal(extracted.target, null, null))
        assertNull(authority.entryProperties(extracted.target).accessedAt)
        assertEquals(1, authority.history(FormKind.ENTRY, extracted.target).size)
        val confirmed = File(path).readBytes()
        assertEquals(extracted, authority.recoverPending(selected.key.source, group, operation))
        assertArrayEquals(confirmed, File(path).readBytes())
        assertEquals(listOf(remaining.source), authority.pendingSources().map { it.source })
        assertEquals(members, manager.host.exchangeView(database).devices.map { it.id }.toSet())
        assertThrows(AndroidException::class.java) { authority.entry(original) }

        authority.lock()
        assertThrows(AndroidException::class.java) { authority.inspectPending(remaining.source) }
        assertThrows(AndroidException::class.java) { authority.revealPending(selected.key.source, origins) }
        manager.documents.close(authority)
        val reopened = manager.documents.open(path, password)
        assertEquals(listOf(remaining.source), reopened.pendingSources().map { it.source })
        assertEquals("PUBLIC late closed second", reopened.inspectPending(remaining.source).entry!!.title)
        assertEquals("PUBLIC late closed first", reopened.inspectPending(selected.key.source).entry!!.title)
        val beforeReplay = File(path).readBytes()
        assertEquals(extracted, reopened.recoverPending(selected.key.source, group, operation))
        assertArrayEquals(beforeReplay, File(path).readBytes())
        assertEquals(1, reopened.history(FormKind.ENTRY, extracted.target).size)
        assertEquals(listOf(extracted.target), reopened.overview("").entries.map { it.id })
        assertTrue(reopened.trashRows().isEmpty())
        assertThrows(AndroidException::class.java) { reopened.entry(original) }
        assertEquals(members, manager.host.exchangeView(database).devices.map { it.id }.toSet())

        // The second real source exercises presentation retries after a lost durable reply.
        val commands = LostExtractionReplyCommands(reopened)
        val state = ReceivedSourcesState(commands) { database }
        state.open()
        state.inspect(state.rows.single())
        assertEquals(remaining.source, state.selected!!.id)
        assertEquals("PUBLIC late closed second", state.entry!!.title)
        val beforeRejectedExtraction = File(path).readBytes()
        state.extract(null) // Native refusal occurs before publication; the user can choose a destination.
        assertTrue(state.error)
        assertFalse(state.busy)
        assertNull(state.attempt)
        assertTrue(state.unresolvedRequests.isEmpty())
        assertNull(commands.dropped)
        assertArrayEquals(beforeRejectedExtraction, File(path).readBytes())
        state.extract(group)
        assertTrue(state.error)
        assertNull(state.extracted)
        val request = state.unresolvedRequests.single()
        assertEquals(remaining.source, request.source.id)
        assertEquals(original, request.entry)
        assertEquals(group, request.group)
        val durable = commands.dropped!!
        assertNotEquals(original, durable.target)
        assertNotEquals(extracted.target, durable.target)
        assertEquals(1, reopened.history(FormKind.ENTRY, durable.target).size)
        assertTrue(reopened.pendingSources().isEmpty())
        state.back()
        assertNull(state.selected)
        assertEquals(request, state.unresolvedRequests.single())
        state.open()
        assertEquals(remaining.source, state.rows.single().id)
        assertEquals(request, state.unresolvedRequests.single())

        state.reset()
        reopened.lock()
        manager.documents.close(reopened)
        commands.session = manager.documents.open(path, password)
        assertEquals(request, state.unresolvedRequests.single())
        state.open()
        assertNull(state.selected)
        assertEquals(remaining.source, state.rows.single().id)
        val beforeLostReplyRetry = File(path).readBytes()
        // No inspection is necessary to resolve the original, immutable operation.
        state.retryExtraction(request)
        assertFalse(state.error)
        assertEquals(durable, state.extracted)
        assertTrue(state.unresolvedRequests.isEmpty())
        assertArrayEquals(beforeLostReplyRetry, File(path).readBytes())
        assertEquals(1, commands.session.history(FormKind.ENTRY, durable.target).size)
        assertEquals(setOf(extracted.target, durable.target), commands.session.overview("").entries.map { it.id }.toSet())
        state.refresh()
        assertEquals(durable, state.extracted)
        assertTrue(state.rows.isEmpty())

        // Refresh must preserve the completed inspector's Open action until explicit departure.
        state.inspect(SourceRow.Closed(remaining))
        assertEquals(durable, state.extracted)
        state.refresh()
        assertEquals(remaining.source, state.selected!!.id)
        assertEquals(durable, state.extracted)
        assertEquals("PUBLIC late closed second", state.entry!!.title)
        assertEquals(members, manager.host.exchangeView(database).devices.map { it.id }.toSet())
        assertThrows(AndroidException::class.java) { commands.session.entry(original) }
    }
}
