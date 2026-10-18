package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.ColorEdit
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.RelayPreference
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.SessionAccess
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Historical metadata comes from actual saved Rust versions, never the current UI projection. */
@RunWith(AndroidJUnit4::class)
class RevisionPropertiesTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val master = "PUBLIC historical properties master"

    @Test fun savedPropertiesStayDistinctReadOnlyAndRequireTheLiveSession() {
        val directory = File(context.noBackupFilesDir, "PUBLIC-revision-properties-${UUID.randomUUID()}")
        try {
            Host.portable(directory.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC historical properties database", null, newOperation(), master)
                    session.beginCreateEntry(publicGroup(session))
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC saved properties title"))
                    session.patchEntry(EntryTextField.PASSWORD, TextEdit.Set("PUBLIC saved properties password"))
                    session.patchTags(listOf("PUBLIC saved tag"))
                    session.patchExpiry("2000000000000")
                    session.setIcon("key-round", newOperation())
                    session.setColors(ColorEdit.Set(0x01020304u), ColorEdit.Set(0x11223344u), newOperation())
                    val saved = session.editor()
                    val receipt = session.save(saved.form.draft, saved.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, receipt.state)
                    val entry = receipt.form.target!!
                    session.deleteForm(receipt.form.draft)
                    val revision = session.history(FormKind.ENTRY, entry).single().id
                    val original = session.entryProperties(entry)
                    assertEquals(listOf("PUBLIC saved tag"), original.tags)
                    assertEquals(2_000_000_000_000L, original.expiresAt)
                    assertEquals("key-round", original.appearance.lucide)
                    assertEquals(0x01020304u, original.appearance.foreground)
                    assertEquals(0x11223344u, original.appearance.background)
                    assertNull(original.appearance.image)

                    session.beginEditEntry(entry)
                    session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC current properties title"))
                    session.patchTags(listOf("PUBLIC current tag", "PUBLIC other current tag"))
                    session.patchExpiry("2100000000000")
                    session.setIcon("bookmark", newOperation())
                    session.setColors(ColorEdit.Set(0xa0b0c0d0u), ColorEdit.Set(0x55667788u), newOperation())
                    val changed = session.editor()
                    assertEquals(SaveState.SAVED,
                        session.save(changed.form.draft, changed.form.revision, newOperation()).state)
                    session.deleteForm(changed.form.draft)
                    val history = session.history(FormKind.ENTRY, entry)
                    assertEquals(2, history.size)
                    val currentRevision = history.single { it.id != revision }.id
                    val current = session.entryProperties(entry)
                    assertEquals(setOf("PUBLIC current tag", "PUBLIC other current tag"), current.tags.toSet())
                    assertEquals(2_100_000_000_000L, current.expiresAt)
                    assertEquals("bookmark", current.appearance.lucide)
                    assertEquals(0xa0b0c0d0u, current.appearance.foreground)
                    assertEquals(0x55667788u, current.appearance.background)
                    assertEquals(original.createdAt, current.createdAt)
                    assertTrue(current.modifiedAt >= original.modifiedAt)
                    assertNotEquals(original, current)

                    val path = manager.catalog().single().path
                    val bytes = File(path).readBytes()
                    assertEquals(original, session.revisionProperties(entry, revision))
                    assertEquals(current, session.revisionProperties(entry, currentRevision))
                    assertEquals(original, session.revisionProperties(entry, revision))
                    assertThrows(AndroidException::class.java) { session.revisionProperties(entry, "") }
                    assertThrows(AndroidException::class.java) { session.revisionProperties("", revision) }
                    assertThrows(AndroidException::class.java) {
                        session.revisionProperties(entry, "PUBLIC nonexistent immutable revision")
                    }
                    assertEquals(history, session.history(FormKind.ENTRY, entry))
                    assertArrayEquals(bytes, File(path).readBytes())

                    // Retain a separate real UniFFI reference before revocation.
                    // The manager destroys its wrapper after confirming worker exit;
                    // the retained wrapper must still reach the revoked native guard.
                    DocumentSession(session.uniffiClonePointer()).use { retained ->
                        assertEquals(SessionAccess.OPEN, retained.access())
                        assertEquals(original, retained.revisionProperties(entry, revision))
                        session.lock()
                        assertEquals(SessionAccess.REVOKED, retained.access())
                        assertThrows(AndroidException.Runtime::class.java) { retained.revisionProperties(entry, revision) }
                        assertThrows(AndroidException.Runtime::class.java) { session.revisionProperties(entry, revision) }
                        assertArrayEquals(bytes, File(path).readBytes())
                        manager.close(session)
                        val destroyed = assertThrows(IllegalStateException::class.java) {
                            session.revisionProperties(entry, revision)
                        }
                        assertEquals("DocumentSession object has already been destroyed", destroyed.message)
                        assertEquals(SessionAccess.REVOKED, retained.access())
                        assertThrows(AndroidException.Runtime::class.java) { retained.revisionProperties(entry, revision) }

                        val reopened = manager.open(path, master)
                        assertEquals(original, reopened.revisionProperties(entry, revision))
                        assertEquals(current, reopened.revisionProperties(entry, currentRevision))
                        assertEquals(history, reopened.history(FormKind.ENTRY, entry))
                        assertArrayEquals(bytes, File(path).readBytes())
                        assertEquals(SessionAccess.OPEN, reopened.access())
                        assertEquals(SessionAccess.REVOKED, retained.access())
                        assertThrows(AndroidException.Runtime::class.java) { retained.revisionProperties(entry, revision) }
                    }
                }
            }
        } finally { assertTrue(directory.deleteRecursively()) }
    }
}
