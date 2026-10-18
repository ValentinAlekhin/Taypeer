package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.DocumentSession
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

/** Actual saved/current payloads, with only delivery to presentation deliberately queued. */
@RunWith(AndroidJUnit4::class)
class EntryComparisonTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val master = "PUBLIC entry comparison master"
    private val savedSecret = "PUBLIC comparison saved password"
    private val currentSecret = "PUBLIC comparison current password"

    private class QueuedCommands(var session: DocumentSession) : SessionCommands {
        private data class Response(val complete: () -> Unit, val fail: () -> Unit)
        private val responses = ArrayDeque<Response>()
        val count get() = responses.size
        var discardedResponses = 0; private set
        override fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
            completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit, discarded: (T) -> Unit): Boolean {
            started()
            val actual = command(session)
            responses.addLast(Response({ completed(actual) }, { discardedResponses++; discarded(actual); failed() }))
            return true
        }
        fun deliver() = responses.removeFirst().complete()
        fun drop() = responses.removeFirst().fail()
    }

    private data class Fixture(val manager: DocumentManager, val session: DocumentSession,
        val entry: String, val revision: String, val path: String)

    private fun fixture(scenario: (Fixture) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-entry-comparison-${UUID.randomUUID()}")
        try {
            Host.portable(profile.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC entry comparison database", null, newOperation(), master)
                    session.beginCreateEntry(publicGroup(session))
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC comparison saved title"))
                    val first = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set(savedSecret))
                    val receipt = session.save(first.form.draft, first.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, receipt.state)
                    val entry = receipt.form.target!!
                    session.deleteForm(receipt.form.draft)
                    val revision = session.history(FormKind.ENTRY, entry).single().id
                    session.beginEditEntry(entry)
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC comparison current title"))
                    val second = session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set(currentSecret))
                    val updated = session.save(second.form.draft, second.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, updated.state)
                    session.deleteForm(updated.form.draft)
                    assertEquals(2, session.history(FormKind.ENTRY, entry).size)
                    scenario(Fixture(manager, session, entry, revision, manager.catalog().single().path))
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    @Test fun savedAndCurrentPasswordActionsAreIndependentAndNeverWriteDatabaseHistory() = fixture { fixture ->
        val before = File(fixture.path).readBytes()
        val commands = QueuedCommands(fixture.session)
        val comparison = EntryComparisonState(commands)
        val copied = mutableListOf<String>()
        comparison.open(fixture.entry, fixture.revision)
        assertTrue(comparison.busy)
        assertNull(comparison.saved)
        assertNull(comparison.current)
        commands.deliver()
        assertEquals("PUBLIC comparison saved title", comparison.saved!!.title)
        assertEquals("PUBLIC comparison current title", comparison.current!!.title)
        assertEquals(true, comparison.saved!!.hasPassword)
        assertEquals(true, comparison.current!!.hasPassword)
        assertNull(comparison.savedPassword)
        assertNull(comparison.currentPassword)

        comparison.reveal(ComparisonSide.SAVED)
        assertNull(comparison.savedPassword)
        commands.deliver()
        assertEquals(savedSecret, comparison.savedPassword)
        assertNull(comparison.currentPassword)
        comparison.reveal(ComparisonSide.CURRENT)
        commands.deliver()
        assertEquals(savedSecret, comparison.savedPassword)
        assertEquals(currentSecret, comparison.currentPassword)
        comparison.reveal(ComparisonSide.SAVED)
        assertNull(comparison.savedPassword)
        assertEquals(currentSecret, comparison.currentPassword)
        assertEquals(0, commands.count)

        comparison.copyPassword(ComparisonSide.SAVED, copied::add)
        assertTrue(copied.isEmpty())
        commands.deliver()
        assertEquals(listOf(savedSecret), copied)
        assertNull(comparison.savedPassword)
        assertEquals(currentSecret, comparison.currentPassword)
        comparison.copyPassword(ComparisonSide.CURRENT, copied::add)
        commands.deliver()
        assertEquals(listOf(savedSecret, currentSecret), copied)
        comparison.hideSecrets()
        assertNull(comparison.savedPassword)
        assertNull(comparison.currentPassword)
        assertArrayEquals(before, File(fixture.path).readBytes())
        assertEquals(2, fixture.session.history(FormKind.ENTRY, fixture.entry).size)
        comparison.close()
        assertFalse(comparison.showing)
        assertNull(comparison.saved)
        assertNull(comparison.current)
    }

    @Test fun lateOrDroppedActualResponsesCannotRepublishAfterHideCloseOrLockReopen() = fixture { fixture ->
        val before = File(fixture.path).readBytes()
        val commands = QueuedCommands(fixture.session)
        val comparison = EntryComparisonState(commands)
        val copied = mutableListOf<String>()
        comparison.open(fixture.entry, fixture.revision)
        commands.deliver()
        comparison.reveal(ComparisonSide.SAVED)
        comparison.hideSecrets()
        commands.deliver()
        assertNull(comparison.savedPassword)
        assertNull(comparison.currentPassword)
        assertFalse(comparison.busy)
        comparison.copyPassword(ComparisonSide.CURRENT, copied::add)
        comparison.hideSecrets()
        commands.deliver()
        assertTrue(copied.isEmpty())
        assertNull(fixture.session.entryProperties(fixture.entry).accessedAt)

        comparison.reveal(ComparisonSide.SAVED)
        comparison.close()
        comparison.open(fixture.entry, fixture.revision)
        commands.deliver() // The previous route's actual saved password.
        assertTrue(comparison.busy)
        assertNull(comparison.saved)
        assertNull(comparison.currentPassword)
        commands.deliver()
        assertFalse(comparison.busy)
        assertEquals("PUBLIC comparison current title", comparison.current!!.title)
        comparison.copyPassword(ComparisonSide.CURRENT, copied::add)
        comparison.close()
        commands.deliver()
        assertTrue(copied.isEmpty())
        assertFalse(comparison.showing)
        assertNull(fixture.session.entryProperties(fixture.entry).accessedAt)

        comparison.open(fixture.entry, fixture.revision)
        commands.drop()
        assertTrue(comparison.error)
        assertFalse(comparison.busy)
        assertNull(comparison.saved)
        comparison.retry()
        commands.deliver()
        assertFalse(comparison.error)
        assertNull(comparison.savedPassword)
        assertNull(comparison.currentPassword)

        comparison.reveal(ComparisonSide.SAVED)
        commands.drop() // The actual Native grant is discarded without drawing or copying.
        assertEquals(2, commands.discardedResponses)
        assertTrue(comparison.error)
        assertNull(fixture.session.entryProperties(fixture.entry).accessedAt)
        comparison.retry()
        commands.deliver()

        comparison.copyPassword(ComparisonSide.SAVED, copied::add)
        fixture.session.lock()
        comparison.close() // DocumentState performs this reset when the session is revoked.
        assertThrows(AndroidException::class.java) { fixture.session.entry(fixture.entry) }
        fixture.manager.close(fixture.session)
        commands.session = fixture.manager.open(fixture.path, master)
        comparison.open(fixture.entry, fixture.revision)
        commands.deliver() // A prepared secret from the revoked generation.
        assertTrue(copied.isEmpty())
        assertTrue(comparison.busy)
        assertNull(comparison.saved)
        assertNull(comparison.current)
        commands.deliver()
        assertFalse(comparison.busy)
        assertEquals("PUBLIC comparison saved title", comparison.saved!!.title)
        assertEquals("PUBLIC comparison current title", comparison.current!!.title)
        assertNull(comparison.savedPassword)
        assertNull(comparison.currentPassword)
        assertNull(commands.session.entryProperties(fixture.entry).accessedAt)
        assertNull(commands.session.revisionProperties(fixture.entry, fixture.revision).accessedAt)
        assertArrayEquals(before, File(fixture.path).readBytes())
        assertEquals(2, commands.session.history(FormKind.ENTRY, fixture.entry).size)
        comparison.close()
    }

    @Test fun hidingOneComparedSideKeepsTheOtherUndrawnGrantEligible() = fixture { fixture ->
        val before = File(fixture.path).readBytes()
        val commands = QueuedCommands(fixture.session)
        val tickets = mutableListOf<SecretDisplayTicket>()
        val comparison = EntryComparisonState(commands) { _, grant, permitted ->
            SecretDisplayTicket(grant, permitted) { accepted -> accepted.use { it.confirm() }; true }.also(tickets::add)
        }
        comparison.open(fixture.entry, fixture.revision)
        commands.deliver()
        comparison.reveal(ComparisonSide.SAVED)
        commands.deliver()
        comparison.reveal(ComparisonSide.CURRENT)
        commands.deliver()
        assertNull(fixture.session.entryProperties(fixture.entry).accessedAt)
        comparison.reveal(ComparisonSide.SAVED)
        assertNull(comparison.savedPassword)
        assertEquals(currentSecret, comparison.currentPassword)
        assertFalse(tickets[0].accept())
        assertTrue(tickets[1].accept()) // Simulates that side's later visible draw, without another reveal.
        assertNotNull(fixture.session.entryProperties(fixture.entry).accessedAt)
        assertFalse(tickets[1].accept())
        comparison.close()
        assertArrayEquals(before, File(fixture.path).readBytes())
        assertEquals(2, fixture.session.history(FormKind.ENTRY, fixture.entry).size)
    }

    @Test fun currentRevealRefreshesActualFieldsAndIncomingRefreshInvalidatesPendingCopy() = fixture { fixture ->
        val commands = QueuedCommands(fixture.session)
        val comparison = EntryComparisonState(commands)
        val copied = mutableListOf<String>()
        comparison.open(fixture.entry, fixture.revision)
        commands.deliver()
        assertEquals("PUBLIC comparison current title", comparison.current!!.title)
        fixture.session.beginEditEntry(fixture.entry)
        fixture.session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC comparison newer live title"))
        val updated = fixture.session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC comparison newer live password"))
        val receipt = fixture.session.save(updated.form.draft, updated.form.revision, newOperation())
        assertEquals(SaveState.SAVED, receipt.state)
        fixture.session.deleteForm(receipt.form.draft)
        val confirmed = File(fixture.path).readBytes()

        comparison.reveal(ComparisonSide.CURRENT)
        assertEquals("PUBLIC comparison current title", comparison.current!!.title)
        commands.deliver()
        assertEquals("PUBLIC comparison newer live title", comparison.current!!.title)
        assertEquals("PUBLIC comparison newer live password", comparison.currentPassword)
        assertEquals("PUBLIC comparison saved title", comparison.saved!!.title)
        comparison.reveal(ComparisonSide.SAVED)
        commands.deliver()
        assertEquals(savedSecret, comparison.savedPassword)

        comparison.copyPassword(ComparisonSide.CURRENT, copied::add)
        comparison.refreshIfOpen()
        assertNull(comparison.savedPassword)
        assertNull(comparison.currentPassword)
        commands.deliver() // Its actual protected payload was invalidated before presentation.
        assertTrue(copied.isEmpty())
        assertNull(comparison.currentPassword)
        assertTrue(comparison.busy) // The queued masked refresh now owns the route.
        assertEquals(1, commands.count)
        commands.deliver()
        assertFalse(comparison.busy)
        assertEquals("PUBLIC comparison newer live title", comparison.current!!.title)
        assertNull(comparison.savedPassword)
        assertNull(comparison.currentPassword)
        assertArrayEquals(confirmed, File(fixture.path).readBytes())
        assertEquals(3, fixture.session.history(FormKind.ENTRY, fixture.entry).size)
        comparison.close()
    }
}
