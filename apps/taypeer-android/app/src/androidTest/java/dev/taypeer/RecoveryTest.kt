package dev.taypeer

import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/** Real encrypted source/candidate, protected profile, private Binder gateway and isolated workers. */
class RecoveryTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val originalPassword = "PUBLIC original recovery password"
    private val recoveredPassword = "PUBLIC new recovery password"
    private val recoveredName = "  PUBLIC recovered exact name  "
    private data class Fixture(val host: Host, val manager: DocumentManager, val session: DocumentSession,
        val source: WorkingCopyView, val entry: String)

    private fun fixture(credentials: CredentialPort = KeystoreCredentials(context), scenario: (Fixture) -> Unit) {
        val directory = File(context.noBackupFilesDir, "PUBLIC-recovery-${UUID.randomUUID()}")
        try {
            Host(directory.path, credentials).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC source recovery database", null, newOperation(), originalPassword)
                    session.beginCreateEntry(publicGroup(session))
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC recovery saved entry"))
                    session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC recovery saved value"))
                    val input = session.editor()
                    val first = session.save(input.form.draft, input.form.revision, newOperation())
                    session.deleteForm(input.form.draft)
                    val entry = first.form.target!!
                    session.beginEditEntry(entry)
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC recovery current entry"))
                    val changed = session.editor()
                    session.save(changed.form.draft, changed.form.revision, newOperation())
                    session.deleteForm(changed.form.draft)
                    scenario(Fixture(host, manager, session, manager.catalog().single(), entry))
                }
            }
        } finally { assertTrue(directory.deleteRecursively()) }
    }

    @Test fun namedRecoveryPublishesOnceAndRequiresRealSourceClosureBeforeCatalogAdoption() = fixture { fixture ->
        val before = File(fixture.source.path).readBytes()
        val history = fixture.session.history(FormKind.ENTRY, fixture.entry)
        val operation = newOperation()
        val publication = fixture.session.recoverWorkingCopy(operation, recoveredName, recoveredPassword)
        assertTrue(publication.size <= 64 * 1024)
        val publicText = publication.toString(Charsets.UTF_8)
        assertFalse(publicText.contains(recoveredName))
        assertFalse(publicText.contains(recoveredPassword))
        assertEquals(fixture.source.path, fixture.host.workingCopies().single().path)
        assertEquals(RecoveryPhase.PROOF_CONFIRMED, fixture.host.pendingRecoveries().single().phase)
        assertThrows(AndroidException.StorageBusy::class.java) { fixture.host.adoptRecovery(publication) }
        assertEquals(fixture.source.path, fixture.host.workingCopies().single().path)
        assertArrayEquals(before, File(fixture.source.path).readBytes())
        val process = fixture.manager.process(fixture.session)
        val copy = fixture.manager.adoptRecovery(publication, fixture.source.database)
        assertTrue(process.exited())
        assertNull(process.persistenceCapability)
        assertNull(process.transfersCapability)
        assertNotEquals(fixture.source.path, copy.path)
        assertNotEquals(fixture.source.root, copy.root)
        assertEquals(fixture.source.database, copy.database)
        assertEquals(copy.path, fixture.host.workingCopies().single().path)
        assertTrue(fixture.host.pendingRecoveries().isEmpty())
        assertEquals(copy.path, fixture.host.adoptRecovery(publication).path)
        val recovered = fixture.manager.open(copy.path, recoveredPassword)
        assertEquals(recoveredName, recovered.overview("").name)
        assertTrue(recovered.overview("").managing)
        assertEquals("PUBLIC recovery current entry", recovered.entry(fixture.entry).title)
        assertEquals(history, recovered.history(FormKind.ENTRY, fixture.entry))
        assertEquals("PUBLIC recovery saved value", recovered.reveal(fixture.entry, history.last().id, null))
        assertArrayEquals(before, File(fixture.source.path).readBytes())
    }

    @Test fun lostProtectedProofAckColdResumeAuthenticatesOnlyTheCandidatePasswordAndKeepsOneRoot() {
        val delegate = KeystoreCredentials(context)
        val fault = AtomicBoolean(true)
        val credentials = object : CredentialPort by delegate {
            override fun set(service: String, account: String, bytes: ByteArray) {
                delegate.set(service, account, bytes)
                if (account.startsWith("state:recovery:") && fault.compareAndSet(true, false)) throw AndroidException.CommitUncertain()
            }
        }
        fixture(credentials) { fixture ->
            val before = File(fixture.source.path).readBytes()
            val history = fixture.session.history(FormKind.ENTRY, fixture.entry)
            val operation = newOperation()
            assertThrows(AndroidException.CommitUncertain::class.java) {
                fixture.session.recoverWorkingCopy(operation, recoveredName, recoveredPassword)
            }
            assertFalse(fault.get())
            val candidates = File(fixture.source.path).parentFile!!.listFiles()!!.filter { it.extension == "taypeer" && it.path != fixture.source.path }
            assertEquals(1, candidates.size)
            val candidateBytes = candidates.single().readBytes()
            assertEquals(operation, fixture.host.pendingRecoveries().single().operation)
            fixture.session.lock(); fixture.manager.close(fixture.session)
            // The original password is deliberately not supplied to this new worker.
            val retry = fixture.manager.openRecovery(fixture.source, recoveredPassword, operation)
            val publication = retry.recoverWorkingCopy(operation, recoveredName, recoveredPassword)
            assertArrayEquals(candidateBytes, candidates.single().readBytes())
            val copy = fixture.manager.adoptRecovery(publication, fixture.source.database)
            assertEquals(candidates.single().path, copy.path)
            assertEquals(1, fixture.host.workingCopies().size)
            val opened = fixture.manager.open(copy.path, recoveredPassword)
            assertEquals(recoveredName, opened.overview("").name)
            assertEquals(history, opened.history(FormKind.ENTRY, fixture.entry))
            assertArrayEquals(before, File(fixture.source.path).readBytes())
        }
    }

    @Test fun abandonedSourceWithoutCandidateRequiresANewExplicitOperationAndCannotPickAnotherPassword() = fixture { fixture ->
        val before = File(fixture.source.path).readBytes()
        fixture.session.lock(); fixture.manager.close(fixture.session)
        val abandoned = newOperation()
        val reader = fixture.manager.openRecovery(fixture.source, originalPassword, abandoned)
        assertEquals(RecoveryPhase.SOURCE_FROZEN, fixture.host.pendingRecoveries().single().phase)
        reader.lock(); fixture.manager.close(reader)
        assertThrows(AndroidException.RecoveryRequiresNewOperation::class.java) {
            fixture.manager.openRecovery(fixture.source, "PUBLIC different abandoned attempt password", abandoned)
        }
        assertEquals(RecoveryPhase.REFUSED_BEFORE_PUBLICATION, fixture.host.pendingRecoveries().single().phase)
        assertEquals(fixture.source.path, fixture.host.workingCopies().single().path)
        val fresh = newOperation()
        assertNotEquals(abandoned, fresh)
        val reopened = fixture.manager.openRecovery(fixture.source, originalPassword, fresh)
        val publication = reopened.recoverWorkingCopy(fresh, recoveredName, recoveredPassword)
        val copy = fixture.manager.adoptRecovery(publication, fixture.source.database)
        assertEquals(recoveredName, fixture.manager.open(copy.path, recoveredPassword).overview("").name)
        assertArrayEquals(before, File(fixture.source.path).readBytes())
    }

    @Test fun lateProtectedProofAfterBackgroundCannotAdoptOrReviveInputAndExplicitCandidateRetryWorks() {
        val delegate = KeystoreCredentials(context)
        val blockOnce = AtomicBoolean(true)
        val proofStored = CountDownLatch(1)
        val releaseProof = CountDownLatch(1)
        val credentials = object : CredentialPort by delegate {
            override fun set(service: String, account: String, bytes: ByteArray) {
                delegate.set(service, account, bytes)
                if (account.startsWith("state:recovery:") && blockOnce.compareAndSet(true, false)) {
                    proofStored.countDown()
                    assertTrue("PUBLIC recovery proof callback was not released", releaseProof.await(20, TimeUnit.SECONDS))
                }
            }
        }
        fixture(credentials) { fixture ->
            val before = File(fixture.source.path).readBytes()
            val history = fixture.session.history(FormKind.ENTRY, fixture.entry)
            val operation = newOperation()
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
            val adoptions = AtomicInteger()
            val result = AtomicReference<WorkingCopyView?>()
            val state = RecoveryState(scope, { fixture.manager }, {}, { copy, password ->
                assertEquals(recoveredPassword, password)
                result.set(copy); adoptions.incrementAndGet()
            })
            try {
                onMain {
                    state.begin(fixture.source, fixture.session, operation)
                    state.updateName(recoveredName); state.updatePassword(recoveredPassword); state.updateRepeated(recoveredPassword)
                    state.submit()
                }
                assertTrue("PUBLIC recovery never reached protected proof publication", proofStored.await(20, TimeUnit.SECONDS))
                onMain { state.lock() }
                fixture.manager.background()
                onMain {
                    assertFalse(state.showing); assertFalse(state.busy); assertNull(state.target)
                    assertTrue(state.name.isEmpty()); assertTrue(state.password.isEmpty()); assertTrue(state.repeated.isEmpty())
                }
                releaseProof.countDown()
                awaitMain { state.pendingLoaded && state.pending.any { it.operation == operation && it.phase == RecoveryPhase.PROOF_CONFIRMED } }
                assertEquals(0, adoptions.get())
                assertEquals(fixture.source.path, fixture.host.workingCopies().single().path)
                assertArrayEquals(before, File(fixture.source.path).readBytes())
                onMain { assertFalse(state.showing); assertNull(state.target); assertTrue(state.password.isEmpty()) }

                // A new constrained worker receives only the candidate's new password and the same ID.
                onMain { state.authenticate(fixture.source, recoveredPassword, operation) }
                awaitMain { state.showing && !state.busy && state.failure == null }
                onMain {
                    assertEquals(operation, state.target!!.operation)
                    state.updateName(recoveredName); state.updatePassword(recoveredPassword); state.updateRepeated(recoveredPassword)
                    state.submit()
                }
                awaitMain { adoptions.get() == 1 && !state.showing && !state.busy }
                val copy = result.get()!!
                assertEquals(copy.path, fixture.host.workingCopies().single().path)
                assertTrue(fixture.host.pendingRecoveries().isEmpty())
                val opened = fixture.manager.open(copy.path, recoveredPassword)
                assertEquals(recoveredName, opened.overview("").name)
                assertEquals(history, opened.history(FormKind.ENTRY, fixture.entry))
                assertArrayEquals(before, File(fixture.source.path).readBytes())
            } finally {
                releaseProof.countDown()
                onMain { state.lock(); scope.cancel() }
            }
        }
    }

    private fun onMain(action: () -> Unit) = InstrumentationRegistry.getInstrumentation().runOnMainSync { action() }
    private fun awaitMain(condition: () -> Boolean) {
        val deadline = android.os.SystemClock.uptimeMillis() + 20_000
        while (true) {
            var completed = false
            onMain { completed = condition() }
            if (completed) return
            assertTrue("PUBLIC recovery state did not reach its expected acknowledged phase", android.os.SystemClock.uptimeMillis() < deadline)
            Thread.sleep(20)
        }
    }
}
