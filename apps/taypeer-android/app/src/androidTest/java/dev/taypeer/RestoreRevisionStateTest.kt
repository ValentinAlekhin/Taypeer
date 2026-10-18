package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.RelayPreference
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Real durable restoration; only delivery of its already committed response is controlled. */
@RunWith(AndroidJUnit4::class)
class RestoreRevisionStateTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val master = "PUBLIC restore retry master"
    private val savedTitle = "PUBLIC restore saved title"
    private val currentTitle = "PUBLIC restore current title"
    private val savedPassword = "PUBLIC restore saved password"
    private val currentPassword = "PUBLIC restore current password"

    private class QueuedCommands(var session: DocumentSession) : SessionCommands {
        private data class Response(val complete: () -> Unit, val fail: () -> Unit)
        private val responses = ArrayDeque<Response>()
        val restored = mutableListOf<String>()
        var executions = 0; private set
        var definitiveRejections = 0; private set
        override fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
            completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit, discarded: (T) -> Unit): Boolean {
            assertTrue(navigate)
            started()
            executions++
            try {
                val actual = command(session)
                restored += actual as String
                responses.addLast(Response({ completed(actual) }, failed))
            } catch (failure: AndroidException) {
                if (failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions) {
                    definitiveRejections++
                    responses.addLast(Response(rejected, failed))
                } else responses.addLast(Response(failed, failed))
            }
            return true
        }
        fun drop() = responses.removeFirst().fail()
        fun deliver() {
            val response = responses.removeFirst()
            // Exercise the older completion wrapper too: a projection exception must
            // never turn the acknowledged native mutation into a failed restoration.
            try { response.complete() } catch (_: Exception) { response.fail() }
        }
    }

    private data class Fixture(val manager: DocumentManager, val session: DocumentSession,
        val database: String, val entry: String, val revision: String, val source: String,
        val destination: String, val alternate: String, val path: String)

    private fun group(session: DocumentSession, name: String): String {
        val started = session.beginMetadata(FormKind.NEW_GROUP, null, null)
        val edited = session.patchMetadata(started.form.draft, FormKind.NEW_GROUP,
            TextEdit.Set(name), TextEdit.Keep)
        val receipt = session.save(edited.form.draft, edited.form.revision, newOperation())
        assertEquals(SaveState.SAVED, receipt.state)
        session.deleteForm(receipt.form.draft)
        return receipt.form.target!!
    }

    private fun fixture(scenario: (Fixture) -> Unit) {
        val directory = File(context.noBackupFilesDir, "PUBLIC-restore-retry-${UUID.randomUUID()}")
        try {
            Host.portable(directory.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC restore retry database", null, newOperation(), master)
                    val source = group(session, "PUBLIC restore source group")
                    val destination = group(session, "PUBLIC restore chosen destination")
                    val alternate = group(session, "PUBLIC restore different destination")
                    session.beginCreateEntry(source)
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set(savedTitle))
                    val saved = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set(savedPassword))
                    val created = session.save(saved.form.draft, saved.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, created.state)
                    val entry = created.form.target!!
                    session.deleteForm(created.form.draft)
                    val revision = session.history(FormKind.ENTRY, entry).single().id
                    session.beginEditEntry(entry)
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set(currentTitle))
                    val changed = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set(currentPassword))
                    assertEquals(SaveState.SAVED,
                        session.save(changed.form.draft, changed.form.revision, newOperation()).state)
                    session.deleteForm(changed.form.draft)
                    assertEquals(2, session.history(FormKind.ENTRY, entry).size)
                    val database = session.databaseId()
                    val path = manager.catalog().single { it.database == database }.path
                    scenario(Fixture(manager, session, database, entry, revision, source, destination, alternate, path))
                }
            }
        } finally { assertTrue(directory.deleteRecursively()) }
    }

    private fun assertRestoredOnce(fixture: Fixture, session: DocumentSession, restored: String) {
        assertEquals(fixture.entry, restored)
        assertEquals(listOf(fixture.entry), session.overview("").entries.map { it.id })
        assertEquals(fixture.destination, session.entry(restored).group)
        assertEquals(savedTitle, session.entry(restored).title)
        assertEquals(savedPassword, session.reveal(restored, null, null))
        assertEquals(3, session.history(FormKind.ENTRY, restored).size)
        assertTrue(session.history(FormKind.ENTRY, restored).any { it.id == fixture.revision && it.name == savedTitle })
        assertTrue(session.history(FormKind.ENTRY, restored).any { it.name == currentTitle })
    }

    @Test fun rejectedDeletedDestinationAllowsCorrectionWithoutChangingBytesOrRepeatingRestoration() = fixture { fixture ->
        val commands = QueuedCommands(fixture.session)
        val opened = mutableListOf<String>()
        val acknowledgements = mutableListOf<Boolean>()
        val state = RestoreRevisionState(commands, { fixture.database }, opened::add)
        fixture.session.trash(FormKind.GROUP, fixture.alternate, newOperation())
        val trashed = fixture.session.trashRows().single { it.target == fixture.alternate }
        val review = fixture.session.prepareTrashPurge(trashed.kind, trashed.target, trashed.generation)
        fixture.session.confirmTrashReview(review.review, newOperation())
        fixture.session.discardTrashReview(review.review)
        val before = File(fixture.path).readBytes()
        val history = fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id }
        val rejectedOperation = newOperation()
        state.confirm(fixture.entry, fixture.revision, fixture.alternate, rejectedOperation, acknowledgements::add)
        val rejected = state.request(fixture.entry, fixture.revision)!!
        assertTrue(state.busy)
        assertEquals(1, commands.definitiveRejections)
        assertEquals(rejectedOperation, rejected.operation)
        assertArrayEquals(before, File(fixture.path).readBytes())
        assertEquals(history, fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id })
        assertEquals(currentTitle, fixture.session.entry(fixture.entry).title)
        assertEquals(fixture.source, fixture.session.entry(fixture.entry).group)
        commands.deliver()
        assertFalse(state.busy)
        assertTrue(state.error)
        assertNull(state.request(fixture.entry, fixture.revision))
        assertTrue(state.unresolvedRequests.isEmpty())
        assertEquals(listOf(false), acknowledgements)
        assertTrue(opened.isEmpty())

        val correctedOperation = newOperation()
        assertNotEquals(rejectedOperation, correctedOperation)
        state.confirm(fixture.entry, fixture.revision, fixture.destination, correctedOperation, acknowledgements::add)
        val corrected = state.request(fixture.entry, fixture.revision)!!
        assertEquals(correctedOperation, corrected.operation)
        assertEquals(fixture.destination, corrected.destination)
        assertRestoredOnce(fixture, fixture.session, commands.restored.single())
        commands.deliver()
        assertFalse(state.busy)
        assertFalse(state.error)
        assertTrue(state.unresolvedRequests.isEmpty())
        assertEquals(listOf(false, true), acknowledgements)
        assertEquals(listOf(fixture.entry), opened)
        val committed = File(fixture.path).readBytes()
        state.retry(rejected, acknowledgements::add)
        state.retry(corrected, acknowledgements::add)
        assertEquals(2, commands.executions)
        assertEquals(listOf(false, true), acknowledgements)
        assertRestoredOnce(fixture, fixture.session, fixture.entry)
        assertArrayEquals(committed, File(fixture.path).readBytes())
    }

    @Test fun lostDurableReplyKeepsExactIntentAcrossRouteDepartureAndLockReopen() = fixture { fixture ->
        val commands = QueuedCommands(fixture.session)
        var database: String? = fixture.database
        val opened = mutableListOf<String>()
        val acknowledgements = mutableListOf<Boolean>()
        val state = RestoreRevisionState(commands, { database }, opened::add)
        val operation = newOperation()
        val before = File(fixture.path).readBytes()
        state.confirm(fixture.entry, fixture.revision, fixture.destination, operation, acknowledgements::add)
        val retained = state.request(fixture.entry, fixture.revision)!!
        val restored = commands.restored.single()
        assertTrue(state.busy)
        assertTrue(opened.isEmpty())
        assertFalse(before.contentEquals(File(fixture.path).readBytes()))
        assertRestoredOnce(fixture, fixture.session, restored)
        commands.drop()
        assertTrue(state.error)
        assertFalse(state.busy)
        assertEquals(listOf(false), acknowledgements)
        assertSame(retained, state.request(fixture.entry, fixture.revision))

        // A later independent move must survive replay of the earlier restoration receipt.
        fixture.session.moveEntry(fixture.entry, fixture.alternate, newOperation())
        assertEquals(fixture.alternate, fixture.session.entry(fixture.entry).group)
        val movedHistory = fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id }
        val committed = File(fixture.path).readBytes()

        // Leaving a history/restore layer does not dispose the DocumentState-owned registry.
        assertEquals(listOf(retained), state.unresolvedRequests)
        val other = fixture.manager.create("PUBLIC unrelated restore database", null, newOperation(), master)
        database = other.databaseId()
        commands.session = other
        state.reset()
        val unrelatedPath = fixture.manager.catalog().single { it.database == database }.path
        val unrelated = File(unrelatedPath).readBytes()
        assertTrue(state.unresolvedRequests.isEmpty())
        assertNull(state.request(fixture.entry, fixture.revision))
        state.retry(retained, acknowledgements::add)
        assertEquals(1, commands.executions)
        assertArrayEquals(unrelated, File(unrelatedPath).readBytes())
        assertArrayEquals(committed, File(fixture.path).readBytes())

        database = null
        state.reset()
        fixture.session.lock()
        fixture.manager.close(fixture.session)
        commands.session = fixture.manager.open(fixture.path, master)
        database = commands.session.databaseId()
        assertSame(retained, state.request(fixture.entry, fixture.revision))
        assertEquals(listOf(retained), state.unresolvedRequests)
        // A changed visible destination and a fresh UI operation must not replace an uncertain request.
        state.confirm(fixture.entry, fixture.revision, fixture.source, newOperation(), acknowledgements::add)
        assertEquals(listOf(restored, restored), commands.restored)
        assertSame(retained, state.request(fixture.entry, fixture.revision))
        assertEquals(fixture.destination, retained.destination)
        assertEquals(operation, retained.operation)
        assertArrayEquals(committed, File(fixture.path).readBytes())
        commands.deliver()
        assertEquals(listOf(false, true), acknowledgements)
        assertEquals(listOf(restored), opened)
        assertNull(state.request(fixture.entry, fixture.revision))
        assertTrue(state.unresolvedRequests.isEmpty())
        assertFalse(state.busy)
        assertFalse(state.error)
        assertEquals(listOf(fixture.entry), commands.session.overview("").entries.map { it.id })
        assertEquals(fixture.alternate, commands.session.entry(fixture.entry).group)
        assertEquals(savedTitle, commands.session.entry(fixture.entry).title)
        assertEquals(savedPassword, commands.session.reveal(fixture.entry, null, null))
        assertEquals(movedHistory, commands.session.history(FormKind.ENTRY, fixture.entry).map { it.id })
        state.retry(retained, acknowledgements::add)
        assertEquals(2, commands.executions)
        assertArrayEquals(committed, File(fixture.path).readBytes())
    }

    @Test fun acknowledgementRemovesIntentBeforeProjectionFailureAndCannotRestoreTwice() = fixture { fixture ->
        val commands = QueuedCommands(fixture.session)
        val acknowledgements = mutableListOf<Boolean>()
        val projected = mutableListOf<String>()
        lateinit var state: RestoreRevisionState
        state = RestoreRevisionState(commands, { fixture.database }) { restored ->
            assertNull(state.request(fixture.entry, fixture.revision))
            assertTrue(state.unresolvedRequests.isEmpty())
            assertEquals(listOf(true), acknowledgements)
            projected += restored
            throw IllegalStateException("PUBLIC restored projection unavailable")
        }
        state.confirm(fixture.entry, fixture.revision, fixture.destination, newOperation(), acknowledgements::add)
        val retained = state.request(fixture.entry, fixture.revision)!!
        val restored = commands.restored.single()
        val committed = File(fixture.path).readBytes()
        commands.deliver()
        assertEquals(listOf(true), acknowledgements)
        assertEquals(listOf(restored), projected)
        assertTrue(state.error)
        assertFalse(state.busy)
        assertNull(state.request(fixture.entry, fixture.revision))
        assertTrue(state.unresolvedRequests.isEmpty())
        state.retry(retained, acknowledgements::add)
        state.reset()
        state.retry(retained, acknowledgements::add)
        assertEquals(1, commands.executions)
        assertEquals(listOf(true), acknowledgements)
        assertArrayEquals(committed, File(fixture.path).readBytes())
        assertRestoredOnce(fixture, fixture.session, restored)
    }
}
