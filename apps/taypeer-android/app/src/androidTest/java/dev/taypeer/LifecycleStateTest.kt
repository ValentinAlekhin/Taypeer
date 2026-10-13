package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Native commands really commit; only their delivery to presentation is queued or lost. */
@RunWith(AndroidJUnit4::class)
class LifecycleStateTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val master = "PUBLIC lifecycle state master"

    private class QueuedCommands(var session: DocumentSession) : SessionCommands {
        private data class Response(val complete: () -> Unit, val fail: () -> Unit)
        private val responses = ArrayDeque<Response>()
        val count get() = responses.size
        var definitiveRejections = 0; private set
        override fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
            completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit, discarded: (T) -> Unit): Boolean {
            started()
            try {
                val actual = command(session)
                responses.addLast(Response({ completed(actual) }, failed))
            } catch (failure: AndroidException) {
                if (failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions) {
                    definitiveRejections++
                    responses.addLast(Response(rejected, failed))
                } else responses.addLast(Response(failed, failed))
            }
            return true
        }
        fun deliver() = responses.removeFirst().complete()
        fun loseReply() = responses.removeFirst().fail()
    }

    private data class Fixture(val manager: DocumentManager, val session: DocumentSession,
        val source: String, val destination: String, val entry: String, val path: String)

    private fun fixture(kind: FormKind, scenario: (Fixture) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-lifecycle-state-${kind.name}-${UUID.randomUUID()}")
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC lifecycle state database", null, newOperation(), master)
                    val source = group(session, "PUBLIC lifecycle state source")
                    val destination = group(session, "PUBLIC lifecycle state destination")
                    session.beginCreateEntry(source)
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC lifecycle state entry"))
                    val ready = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC lifecycle state protected value"))
                    val saved = session.save(ready.form.draft, ready.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, saved.state)
                    session.deleteForm(saved.form.draft)
                    scenario(Fixture(manager, session, source, destination, saved.form.target!!, manager.catalog().single().path))
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    private fun group(session: DocumentSession, name: String): String {
        val initial = session.beginMetadata(FormKind.NEW_GROUP, null, null)
        val ready = session.patchMetadata(initial.form.draft, FormKind.NEW_GROUP, TextEdit.Set(name), TextEdit.Keep)
        val saved = session.save(ready.form.draft, ready.form.revision, newOperation())
        assertEquals(SaveState.SAVED, saved.state)
        session.deleteForm(saved.form.draft)
        return saved.form.target!!
    }

    private fun purge(session: DocumentSession, kind: FormKind, target: String) {
        session.trash(kind, target, newOperation())
        val row = session.trashRows().single { it.kind == kind && it.target == target }
        val reviewed = session.prepareTrashPurge(row.kind, row.target, row.generation)
        session.confirmTrashReview(reviewed.review, newOperation())
        session.discardTrashReview(reviewed.review)
    }

    @Test fun rejectedDeletedDestinationClearsOnlyRejectedIntentAndAllowsOneCorrectedClone() = fixture(FormKind.ENTRY) { fixture ->
        val commands = QueuedCommands(fixture.session)
        val destinations = mutableListOf<String?>()
        val state = DocumentLifecycleState(commands, { fixture.session.databaseId() }, destinations::add)
        state.openAction(ObjectActionSelection(FormKind.ENTRY, fixture.entry, ObjectAction.CLONE, "PUBLIC clone source", fixture.source))
        commands.deliver()
        purge(fixture.session, FormKind.GROUP, fixture.destination)
        val before = File(fixture.path).readBytes()
        val history = fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id }
        val rejectedOperation = newOperation()
        state.confirmAction(fixture.destination, "PUBLIC corrected clone title", rejectedOperation)
        assertTrue(state.busy)
        assertEquals(1, commands.definitiveRejections)
        val rejected = state.unresolvedActions.single()
        assertEquals(rejectedOperation, rejected.operation)
        assertArrayEquals(before, File(fixture.path).readBytes())
        assertEquals(history, fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id })
        commands.deliver()
        assertFalse(state.busy)
        assertTrue(state.error)
        assertNull(state.attempt)
        assertNotNull(state.selection)
        assertTrue(state.unresolvedActions.isEmpty())
        assertTrue(destinations.isEmpty())

        val correctedOperation = newOperation()
        assertNotEquals(rejectedOperation, correctedOperation)
        state.confirmAction(fixture.source, "PUBLIC corrected clone title", correctedOperation)
        val corrected = state.unresolvedActions.single()
        assertEquals(correctedOperation, corrected.operation)
        assertEquals(fixture.source, corrected.destination)
        commands.deliver()
        assertFalse(state.busy)
        assertFalse(state.error)
        assertTrue(state.unresolvedActions.isEmpty())
        assertEquals(listOf(fixture.source), destinations)
        val clone = fixture.session.overview("").entries.single { it.id != fixture.entry }
        assertEquals(fixture.source, clone.group)
        assertEquals("PUBLIC corrected clone title", clone.title)
        assertEquals(1, fixture.session.history(FormKind.ENTRY, clone.id).size)
        assertEquals(history, fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id })
        val committed = File(fixture.path).readBytes()
        state.retry(rejected)
        state.retry(corrected)
        assertEquals(0, commands.count)
        assertArrayEquals(committed, File(fixture.path).readBytes())
        assertEquals(2, fixture.session.overview("").entries.size)
    }

    @Test fun lostCloneReplyRetainsExactIntentAcrossBackLockOtherDatabaseAndPurgedSource() {
        for (kind in listOf(FormKind.ENTRY, FormKind.GROUP)) fixture(kind) { fixture ->
            val commands = QueuedCommands(fixture.session)
            var database: String? = fixture.session.databaseId()
            val originalDatabase = database!!
            val destinations = mutableListOf<String?>()
            val state = DocumentLifecycleState(commands, database = { database }, selectDestination = destinations::add)
            val target = if (kind == FormKind.ENTRY) fixture.entry else fixture.source
            val selected = ObjectActionSelection(kind, target, ObjectAction.CLONE, "PUBLIC original caption", fixture.source)
            val before = fixture.session.overview("")
            state.openAction(selected)
            commands.deliver()
            val operation = newOperation()
            state.confirmAction(fixture.destination, "PUBLIC retained clone title", operation)
            val committed = fixture.session.overview("")
            val copiedEntries = committed.entries.map { it.id }.toSet() - before.entries.map { it.id }.toSet()
            val copiedGroups = committed.groups.map { it.id }.toSet() - before.groups.map { it.id }.toSet()
            assertEquals(1, copiedEntries.size)
            assertEquals(if (kind == FormKind.GROUP) 1 else 0, copiedGroups.size)
            val copied = copiedEntries.single()
            assertEquals(1, fixture.session.history(FormKind.ENTRY, copied).size)
            assertEquals("PUBLIC lifecycle state protected value", fixture.session.reveal(copied, null, null))
            assertTrue(state.busy)
            commands.loseReply()
            assertTrue(state.error)
            assertFalse(state.busy)
            assertTrue(destinations.isEmpty())
            val request = state.unresolvedActions.single()
            assertSame(request, state.attempt)
            assertEquals(operation, request.operation)
            assertEquals(fixture.destination, request.destination)
            assertEquals("PUBLIC retained clone title", request.name)

            state.closeAction()
            assertNull(state.selection)
            assertNull(state.attempt)
            assertSame(request, state.unresolvedActions.single())
            state.openAction(selected.copy(name = "PUBLIC changed caption", destination = null))
            commands.deliver()
            assertSame(request, state.attempt)
            assertEquals(request.name, state.selection!!.name)
            assertEquals(request.destination, state.selection!!.destination)
            state.closeAction()

            purge(fixture.session, kind, target)
            assertThrows(AndroidException::class.java) {
                if (kind == FormKind.ENTRY) fixture.session.entry(target)
                else fixture.session.beginMetadata(FormKind.GROUP, target, null)
            }
            val confirmed = File(fixture.path).readBytes()
            fixture.session.lock()
            database = null
            state.reset()
            assertNull(state.attempt)
            assertNull(state.selection)
            assertTrue(state.unresolvedActions.isEmpty())
            fixture.manager.close(fixture.session)

            val other = fixture.manager.create("PUBLIC different lifecycle database", null, newOperation(), master)
            commands.session = other
            database = other.databaseId()
            val otherPath = fixture.manager.catalog().single { it.database == database }.path
            val otherBefore = File(otherPath).readBytes()
            assertTrue(state.unresolvedActions.isEmpty())
            state.retry(request)
            assertEquals(0, commands.count)
            assertTrue(other.overview("").entries.isEmpty())
            assertArrayEquals(otherBefore, File(otherPath).readBytes())
            fixture.manager.close(other)

            commands.session = fixture.manager.open(fixture.path, master)
            database = commands.session.databaseId()
            assertEquals(originalDatabase, database)
            assertSame(request, state.unresolvedActions.single())
            state.retry(request) // Does not inspect a source that has already been purged.
            assertTrue(state.busy)
            assertSame(request, state.attempt)
            assertArrayEquals(confirmed, File(fixture.path).readBytes())
            commands.deliver()
            assertFalse(state.busy)
            assertFalse(state.error)
            assertNull(state.selection)
            assertNull(state.attempt)
            assertTrue(state.unresolvedActions.isEmpty())
            assertEquals(listOf(fixture.destination), destinations)
            assertEquals(listOf(copied), commands.session.overview("").entries.map { it.id })
            assertEquals(1, commands.session.history(FormKind.ENTRY, copied).size)
            assertArrayEquals(confirmed, File(fixture.path).readBytes())
            state.retry(request) // A confirmed request cannot be resubmitted by a stale UI callback.
            assertEquals(0, commands.count)
        }
    }

    @Test fun lostMoveReplyAndLateCallbacksCannotReapplyAnOldDestinationAfterReopen() {
        for (kind in listOf(FormKind.ENTRY, FormKind.GROUP)) fixture(kind) { fixture ->
            val commands = QueuedCommands(fixture.session)
            var database: String? = fixture.session.databaseId()
            val destinations = mutableListOf<String?>()
            val state = DocumentLifecycleState(commands, database = { database }, selectDestination = destinations::add)
            val target = if (kind == FormKind.ENTRY) fixture.entry else fixture.source
            val selected = ObjectActionSelection(kind, target, ObjectAction.MOVE, "PUBLIC move caption", null)
            state.openAction(selected)
            commands.deliver()
            state.confirmAction(fixture.destination, "", newOperation())
            commands.loseReply()
            val request = state.unresolvedActions.single()
            assertTrue(state.error)
            if (kind == FormKind.ENTRY) {
                assertEquals(fixture.destination, fixture.session.entry(target).group)
                assertEquals(2, fixture.session.history(kind, target).size)
                fixture.session.moveEntry(target, fixture.source, newOperation())
            } else {
                assertEquals(fixture.destination, fixture.session.overview("").groups.single { it.id == target }.parent)
                fixture.session.moveGroup(target, null, newOperation())
            }
            val latestHistory = fixture.session.history(kind, target).map { it.id }
            val confirmed = File(fixture.path).readBytes()
            state.closeAction()
            assertSame(request, state.unresolvedActions.single())
            state.retry(request) // Native returns the first move's receipt, preserving the later move.
            assertArrayEquals(confirmed, File(fixture.path).readBytes())
            fixture.session.lock()
            database = null
            state.reset()
            fixture.manager.close(fixture.session)
            commands.session = fixture.manager.open(fixture.path, master)
            database = commands.session.databaseId()
            state.openAction(selected)
            commands.deliver() // A prepared old-generation completion must not clear the new busy state.
            assertTrue(state.busy)
            assertTrue(destinations.isEmpty())
            assertSame(request, state.unresolvedActions.single())
            assertNull(state.selection)
            commands.deliver()
            assertFalse(state.busy)
            assertSame(request, state.attempt)
            state.confirmAction(fixture.source, "PUBLIC changed move name", newOperation())
            assertSame(request, state.attempt)
            assertArrayEquals(confirmed, File(fixture.path).readBytes())
            commands.deliver()
            assertFalse(state.busy)
            assertTrue(state.unresolvedActions.isEmpty())
            assertEquals(listOf(fixture.destination), destinations)
            if (kind == FormKind.ENTRY) assertEquals(fixture.source, commands.session.entry(target).group)
            else assertNull(commands.session.overview("").groups.single { it.id == target }.parent)
            assertEquals(latestHistory, commands.session.history(kind, target).map { it.id })
            assertArrayEquals(confirmed, File(fixture.path).readBytes())
        }
    }
}
