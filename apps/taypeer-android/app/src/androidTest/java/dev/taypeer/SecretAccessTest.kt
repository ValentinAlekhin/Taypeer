package dev.taypeer

import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean

/** Actual Android host/isolated worker/Keystore boundary, separate from rendering acceptance tests. */
class SecretAccessTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC access master password"
    private data class Fixture(val manager: DocumentManager, val session: DocumentSession, val entry: String,
        val revision: String, val protected: String, val plain: String, val path: String)

    private fun fixture(credentials: CredentialPort = KeystoreCredentials(context), scenario: (Fixture) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-secret-access-${UUID.randomUUID()}")
        try {
            Host(profile.path, credentials).use { host ->
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
                    scenario(Fixture(manager, session, entry, revision, protected, plain, manager.catalog().single().path))
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

    @Test fun realCredentialPublicationFailureRetainsTheExactGrantForConfirmOnlyRetry() {
        val delegate = KeystoreCredentials(context)
        val fail = AtomicBoolean(true)
        val credentials = object : CredentialPort by delegate {
            override fun set(service: String, account: String, bytes: ByteArray) {
                delegate.set(service, account, bytes)
                if (account.startsWith("state:entry-access:") && fail.compareAndSet(true, false)) throw AndroidException.CommitUncertain()
            }
        }
        fixture(credentials) { fixture ->
            val before = File(fixture.path).readBytes()
            val history = fixture.session.history(FormKind.ENTRY, fixture.entry)
            val issued = fixture.session.revealForAccess(fixture.entry, null, null)
            issued.grant.use { grant ->
                assertThrows(AndroidException.CommitUncertain::class.java) { grant.confirm() }
                assertFalse(fail.get())
                val confirmed = grant.confirm()
                assertEquals(confirmed, fixture.session.entryProperties(fixture.entry).accessedAt)
                assertEquals(confirmed, grant.confirm())
            }
            assertEquals(history, fixture.session.history(FormKind.ENTRY, fixture.entry))
            assertArrayEquals(before, File(fixture.path).readBytes())
        }
    }
}
