package dev.taypeer

import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.SelectedDescriptors
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Real Binder/isolated workers; only delivery of a completed Native reply is controlled. */
@RunWith(AndroidJUnit4::class)
class PolicyStateTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val master = "PUBLIC policy current master password"
    private class QueuedCommands(var session: DocumentSession) : SessionCommands {
        private data class Reply(val deliver: () -> Unit, val drop: () -> Unit)
        private val replies = ArrayDeque<Reply>()
        var executions = 0; private set
        var writes = 0; private set
        var lastFailureType: String? = null; private set
        override fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
            completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit, discarded: (T) -> Unit): Boolean {
            assertFalse(navigate)
            started(); executions++; lastFailureType = null
            try {
                val value = command(session)
                if (value == Unit) writes++
                replies.addLast(Reply({ completed(value) }, failed))
            } catch (failure: AndroidException) {
                lastFailureType = failure.javaClass.simpleName
                val definitive = failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions
                replies.addLast(Reply(if (definitive) rejected else failed, failed))
            }
            return true
        }
        fun deliver() = replies.removeFirst().deliver()
        fun drop() = replies.removeFirst().drop()
    }
    private class Fixture(val manager: DocumentManager, val session: DocumentSession,
        val source: PolicySource, val entry: String, val path: File)
    private fun fixture(block: (Fixture) -> Unit) {
        val directory = File(context.noBackupFilesDir, "PUBLIC-policy-${UUID.randomUUID()}")
        val selected = File(context.cacheDir, "PUBLIC-selected-fixtures/PUBLIC-${UUID.randomUUID()}.bin")
        check(selected.parentFile!!.isDirectory || selected.parentFile!!.mkdirs())
        try {
            Host.portable(directory.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC policy database", null, newOperation(), master)
                    val group = session.beginMetadata(FormKind.NEW_GROUP, null, null)
                    val named = session.patchMetadata(group.form.draft, FormKind.NEW_GROUP,
                        TextEdit.Set("PUBLIC policy group"), TextEdit.Keep)
                    val groupSaved = session.save(named.form.draft, named.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, groupSaved.state)
                    session.deleteForm(groupSaved.form.draft)
                    session.beginCreateEntry(groupSaved.form.target!!)
                    val entry = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC policy entry"))
                    selected.writeBytes("PUBLIC retained policy attachment".toByteArray())
                    val attached = session.importAttachment(entry.form.draft, "PUBLIC policy.bin", null, newOperation(),
                        SelectedDescriptors.openInput(context, Uri.parse("content://dev.taypeer.publicselected/${selected.name}")))
                    val saved = session.save(attached.form.draft, attached.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, saved.state)
                    session.deleteForm(saved.form.draft)
                    val copy = manager.catalog().single { it.database == session.databaseId() }
                    block(Fixture(manager, session, PolicySource(copy.database, copy.path, session.generation()), saved.form.target!!, File(copy.path)))
                }
            }
        } finally { selected.delete(); assertTrue(directory.deleteRecursively()) }
    }

    @Test fun actualCalibrationAndSupersededPreviewPreserveCiphertextHistoryDraftAndUsage() = fixture { fixture ->
        val commands = QueuedCommands(fixture.session)
        val state = PolicyState(commands, { fixture.source }, { true }, {})
        fixture.session.beginEditEntry(fixture.entry)
        val draft = fixture.session.patchEntry(EntryTextField.NOTES, TextEdit.Set("PUBLIC unsaved calibration notes"))
        val before = fixture.path.readBytes()
        val history = fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id }
        state.show(PolicyPage.PROTECTION); commands.deliver()
        val details = state.details!!
        assertEquals("PUBLIC retained policy attachment".toByteArray().size.toULong(), details.usage.attachmentBytes)
        assertEquals(before.size.toULong(), details.usage.fileBytes)
        assertEquals(65_536u, details.parameters.memoryKib)
        assertEquals(1u, details.parameters.parallelism)
        assertTrue(details.passwordDerivationMs!! > 0uL)
        state.calibrate()
        assertTrue(state.calibrating)
        commands.deliver()
        assertEquals(details.policy.kdfTargetMs, state.preview!!.targetMs)
        assertTrue(state.preview!!.elapsedMs > 0uL)
        assertTrue(state.preview!!.parameters.iterations in 3u..256u)
        state.calibrate()
        state.selectTarget(2000u)
        commands.deliver()
        assertNull(state.preview)
        assertFalse(state.calibrating)
        state.calibrate()
        state.close()
        commands.deliver()
        assertNull(state.page); assertNull(state.details); assertNull(state.preview)
        assertArrayEquals(before, fixture.path.readBytes())
        assertEquals(history, fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id })
        assertEquals(draft.form.draft, fixture.session.activeForm()!!.draft)
        assertEquals("PUBLIC unsaved calibration notes", fixture.session.editor().notes)
        assertEquals(0, commands.writes)
        assertNull(fixture.session.entryProperties(fixture.entry).accessedAt)
        fixture.session.deleteForm(draft.form.draft)
        fixture.manager.close(fixture.session)
        val reopened = fixture.manager.open(fixture.path.path, master)
        assertArrayEquals(before, fixture.path.readBytes())
        assertEquals(history, reopened.history(FormKind.ENTRY, fixture.entry).map { it.id })
        assertEquals(details.usage.attachmentBytes, reopened.policyDetails().usage.attachmentBytes)
    }

    @Test fun typedWrongPasswordKeepsRawInputAndCorrectionPublishesOnlyOnePolicy() = fixture { fixture ->
        val commands = QueuedCommands(fixture.session)
        var selected: PolicySource? = fixture.source
        var confirmed = 0
        val state = PolicyState(commands, { selected }, { selected != null }, { confirmed++ })
        state.show(PolicyPage.PROTECTION); commands.deliver()
        val before = fixture.path.readBytes()
        val history = fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id }
        state.selectTarget(2000u)
        state.editPassword("PUBLIC wrong current master")
        state.apply()
        val rejected = state.retained!!
        commands.deliver()
        assertEquals(PolicyFailure.REJECTED, state.failure)
        assertNull(state.retained)
        assertTrue(state.editable)
        assertEquals("PUBLIC wrong current master", state.password)
        assertArrayEquals(before, fixture.path.readBytes())
        assertEquals(history, fixture.session.history(FormKind.ENTRY, fixture.entry).map { it.id })
        assertEquals(1000u, fixture.session.policy().kdfTargetMs)
        state.editPassword(master)
        state.apply()
        val corrected = state.retained!!
        assertNotEquals(rejected.operation, corrected.operation)
        commands.deliver()
        // A changed KDF target rotates the real epoch. Its publication can revoke
        // this worker before its ACK is delivered; the same operation must then
        // reconcile after actual exit, rather than claiming a second policy write.
        val needsReceipt = confirmed == 0
        if (needsReceipt) {
            assertEquals("failureType=${commands.lastFailureType}, busy=${state.busy}, page=${state.page}",
                PolicyFailure.UNCERTAIN, state.failure)
            assertSame(corrected, state.retained)
            assertFalse(state.editable); assertFalse(state.closeEnabled)
        } else {
            assertEquals(1, confirmed)
            assertNull(state.page); assertNull(state.retained); assertEquals("", state.password)
        }
        DocumentSession(fixture.session.uniffiClonePointer()).use { oldWorker ->
            selected = null; state.reset()
            assertEquals("", state.password)
            fixture.manager.close(fixture.session)
            assertEquals(SessionAccess.REVOKED, oldWorker.access())
            assertThrows(AndroidException.Runtime::class.java) { oldWorker.policyDetails() }
        }
        val durable = fixture.path.readBytes()
        assertFalse(before.contentEquals(durable))
        val reopened = fixture.manager.open(fixture.path.path, master)
        assertNotEquals(fixture.source.generation, reopened.generation())
        commands.session = reopened
        selected = fixture.source.copy(generation = reopened.generation())
        if (needsReceipt) {
            val executions = commands.executions
            state.show(PolicyPage.PROTECTION)
            assertEquals(executions, commands.executions) // Retry precedes any projection read.
            assertNull(state.details); assertEquals("", state.password)
            assertSame(corrected, state.retained)
            state.retry(); commands.deliver()
        } else {
            // The acknowledged path also proves the durable receipt needs no
            // old password and neither rotates again nor changes the ciphertext.
            reopened.setPolicy(corrected.value, null, corrected.operation)
        }
        assertEquals(1, confirmed); assertEquals(1, commands.writes)
        assertNull(state.page); assertNull(state.retained); assertEquals("", state.password)
        val acknowledged = reopened.policyDetails()
        assertEquals(2000u, acknowledged.policy.kdfTargetMs)
        assertTrue(acknowledged.passwordDerivationMs!! > 0uL)
        assertEquals(history, reopened.history(FormKind.ENTRY, fixture.entry).map { it.id })
        assertArrayEquals(durable, fixture.path.readBytes())
    }

    @Test fun lostDurableReplyRetriesExactIntentBeforeInspectionAfterLockAndCannotTouchAnotherCopy() = fixture { fixture ->
        val commands = QueuedCommands(fixture.session)
        var selected: PolicySource? = fixture.source
        var confirmed = 0
        val state = PolicyState(commands, { selected }, { selected != null }, { confirmed++ })
        state.show(PolicyPage.LIMITS); commands.deliver()
        state.editAttachment("1.5"); state.editTotal("3")
        state.apply()
        val retained = state.retained!!
        val durable = fixture.path.readBytes()
        commands.drop()
        assertEquals(PolicyFailure.UNCERTAIN, state.failure)
        assertFalse(state.editable); assertFalse(state.closeEnabled)
        state.editTotal("4"); state.close()
        assertEquals("3", state.total); assertEquals(PolicyPage.LIMITS, state.page)
        val other = fixture.manager.create("PUBLIC unrelated policy database", null, newOperation(), master)
        val otherCopy = fixture.manager.catalog().single { it.database == other.databaseId() }
        val otherBefore = File(otherCopy.path).readBytes()
        selected = PolicySource(otherCopy.database, otherCopy.path, other.generation())
        commands.session = other; state.reset()
        val executions = commands.executions
        state.retry()
        assertEquals(executions, commands.executions)
        assertNull(state.retained)
        assertArrayEquals(otherBefore, File(otherCopy.path).readBytes())
        selected = null; state.reset()
        fixture.manager.close(fixture.session)
        val reopened = fixture.manager.open(fixture.path.path, master)
        commands.session = reopened
        selected = fixture.source.copy(generation = reopened.generation())
        state.show(PolicyPage.PROTECTION)
        assertEquals(executions, commands.executions) // No read can block the exact receipt retry.
        assertNull(state.details); assertEquals("", state.password)
        assertSame(retained, state.retained)
        assertEquals(PolicyPage.LIMITS, state.page)
        state.retry(); commands.deliver()
        assertEquals(1, confirmed)
        assertNull(state.page); assertNull(state.retained)
        assertEquals(retained.value.attachmentBytes, reopened.policy().attachmentBytes)
        assertEquals(retained.value.totalAttachmentBytes, reopened.policy().totalAttachmentBytes)
        assertArrayEquals(durable, fixture.path.readBytes())
        assertArrayEquals(otherBefore, File(otherCopy.path).readBytes())
        assertEquals(1, reopened.history(FormKind.ENTRY, fixture.entry).size)
        assertNull(reopened.entryProperties(fixture.entry).accessedAt)
    }
}
