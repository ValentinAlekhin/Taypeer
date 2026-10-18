package dev.taypeer

import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.UUID

/** Actual Android host/isolated worker/portable state boundary. */
class SecretAccessTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC access master password"
    private data class Fixture(val manager: DocumentManager, val session: DocumentSession, val entry: String,
        val revision: String, val protected: String, val plain: String, val path: String, val profile: File)

    private fun fixture(scenario: (Fixture) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-secret-access-${UUID.randomUUID()}")
        try {
            Host.portable(profile.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC access database", null, newOperation(), password)
                    session.beginCreateEntry(publicGroup(session))
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC access entry"))
                    session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC saved access password"))
                    session.patchAttribute(null, "PUBLIC protected attribute", TextEdit.Set("PUBLIC protected saved value"), true, false)
                    session.patchAttribute(null, "PUBLIC ordinary attribute", TextEdit.Set("PUBLIC ordinary value"), false, false)
                    val edited = session.editor()
                    val receipt = session.save(edited.form.draft, edited.form.revision, newOperation())
                    val entry = receipt.form.target!!
                    val protected = session.editor().attributes.single { it.protected }.id!!
                    val plain = session.editor().attributes.single { !it.protected }.id!!
                    session.deleteForm(receipt.form.draft)
                    val revision = session.history(FormKind.ENTRY, entry).single().id
                    session.beginEditEntry(entry)
                    session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC current access password"))
                    val latest = session.editor()
                    session.save(latest.form.draft, latest.form.revision, newOperation())
                    session.deleteForm(latest.form.draft)
                    scenario(Fixture(manager, session, entry, revision, protected, plain, manager.catalog().single().path, profile))
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    @Test fun issuedViewsWriteNothingAndConfirmedSavedScopesShareOneLocalEntryMark() = fixture { fixture ->
        val session = fixture.session
        val before = File(fixture.path).readBytes()
        val history = session.history(FormKind.ENTRY, fixture.entry)
        assertNull(session.entryProperties(fixture.entry).accessedAt)
        assertNull(session.revisionProperties(fixture.entry, fixture.revision).accessedAt)
        assertEquals("PUBLIC current access password", session.reveal(fixture.entry, null, null))
        assertThrows(AndroidException.InvalidInput::class.java) { session.revealForAccess(fixture.entry, null, fixture.plain) }
        val current = session.revealForAccess(fixture.entry, null, null)
        assertEquals("PUBLIC current access password", current.value)
        assertNull(session.entryProperties(fixture.entry).accessedAt)
        current.grant.close() // A caller that did not accept the value never confirms.
        assertNull(session.entryProperties(fixture.entry).accessedAt)
        val saved = session.revealForAccess(fixture.entry, fixture.revision, null)
        assertEquals("PUBLIC saved access password", saved.value)
        saved.grant.use { grant ->
            val confirmed = grant.confirm()
            assertEquals(confirmed, session.entryProperties(fixture.entry).accessedAt)
            assertEquals(confirmed, session.revisionProperties(fixture.entry, fixture.revision).accessedAt)
            assertEquals(confirmed, grant.confirm())
        }
        val protected = session.revealForAccess(fixture.entry, fixture.revision, fixture.protected)
        assertEquals("PUBLIC protected saved value", protected.value)
        protected.grant.use { grant ->
            val confirmed = grant.confirm()
            assertEquals(confirmed, session.entryProperties(fixture.entry).accessedAt)
            assertEquals(confirmed, session.revisionProperties(fixture.entry, fixture.revision).accessedAt)
        }
        assertEquals(history, session.history(FormKind.ENTRY, fixture.entry))
        assertArrayEquals(before, File(fixture.path).readBytes())
    }

    @Test fun acceptedGrantCanConfirmAfterLockAndEncryptedReopenWithoutRevivingWorkerAccess() = fixture { fixture ->
        val before = File(fixture.path).readBytes()
        val session = fixture.session
        val history = session.history(FormKind.ENTRY, fixture.entry)
        val accepted = session.revealForAccess(fixture.entry, fixture.revision, null)
        assertEquals("PUBLIC saved access password", accepted.value)
        session.lock()
        assertThrows(AndroidException.Runtime::class.java) { session.revealForAccess(fixture.entry, null, null) }
        fixture.manager.close(session)
        val confirmed = accepted.grant.use { it.confirm() }
        val reopened = fixture.manager.open(fixture.path, password)
        assertEquals(confirmed, reopened.entryProperties(fixture.entry).accessedAt)
        assertEquals(confirmed, reopened.revisionProperties(fixture.entry, fixture.revision).accessedAt)
        assertEquals(history, reopened.history(FormKind.ENTRY, fixture.entry))
        assertArrayEquals(before, File(fixture.path).readBytes())
    }

    @Test fun changedPortableStateRetainsTheExactGrantForConfirmOnlyRetry() = fixture { fixture ->
        val before = File(fixture.path).readBytes()
        val history = fixture.session.history(FormKind.ENTRY, fixture.entry)
        val issued = fixture.session.revealForAccess(fixture.entry, null, null)
        val state = publicPortableStates(fixture.profile).keys.single()
        val protected = state.readBytes()
        issued.grant.use { grant ->
            // Change only this PUBLIC fixture's encrypted local state. The real FileStore
            // must reject the stale generation before confirming an access mark.
            val changed = protected.copyOf().also { it[it.lastIndex] = (it.last().toInt() xor 1).toByte() }
            state.writeBytes(changed)
            try {
                assertThrows(AndroidException.ProfileInvalid::class.java) { grant.confirm() }
                assertNull(fixture.session.entryProperties(fixture.entry).accessedAt)
            } finally { state.writeBytes(protected) }
            val confirmed = grant.confirm()
            assertEquals(confirmed, fixture.session.entryProperties(fixture.entry).accessedAt)
            assertEquals(confirmed, grant.confirm())
        }
        fixture.session.lock(); fixture.manager.close(fixture.session)
        val reopened = fixture.manager.open(fixture.path, password)
        assertNotNull(reopened.entryProperties(fixture.entry).accessedAt)
        assertEquals(history, reopened.history(FormKind.ENTRY, fixture.entry))
        assertArrayEquals(before, File(fixture.path).readBytes())
    }
}
