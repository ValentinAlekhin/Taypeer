package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.IconView
import dev.taypeer.bridge.RelayPreference
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.TextEdit
import dev.taypeer.platform.DocumentManager
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Actual isolated workers distinguish a retained icon form from the saved group selection. */
@RunWith(AndroidJUnit4::class)
class GroupIconTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC group icon master"

    @Test fun groupIconSelectionSurvivesSaveLockAndReopenWithoutCommittingUnfinishedChanges() {
        val profile = File(context.noBackupFilesDir, "PUBLIC-group-icon-${UUID.randomUUID()}")
        try {
            Host.portable(profile.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { manager ->
                    val session = manager.create("PUBLIC group icon database", null, newOperation(), password)
                    assertNotEquals(android.os.Process.myPid(), manager.process(session).pid)
                    val database = session.beginMetadata(FormKind.DATABASE, null, null)
                    assertNull(database.icon)
                    session.deleteForm(database.form.draft)

                    val initial = session.beginMetadata(FormKind.NEW_GROUP, null, null)
                    assertEquals(IconView.Default, initial.icon)
                    session.patchMetadata(initial.form.draft, FormKind.NEW_GROUP,
                        TextEdit.Set("PUBLIC icon group"), TextEdit.Keep)
                    val selected = session.patchGroupIcon(initial.form.draft, TextEdit.Set("server"))
                    assertEquals(initial.form.draft, selected.form.draft)
                    assertTrue(selected.form.revision > initial.form.revision)
                    assertTrue(selected.form.dirty)
                    assertEquals(IconView.Lucide("server"), selected.icon)
                    assertEquals(selected.form, session.metadata(selected.form.draft).form)
                    assertTrue(session.overview("").groups.isEmpty())
                    val saved = session.save(selected.form.draft, selected.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, saved.state)
                    val group = saved.form.target!!
                    session.deleteForm(saved.form.draft)
                    assertEquals(IconView.Lucide("server"), session.overview("").groups.single().icon)
                    assertEquals(1, session.history(FormKind.GROUP, group).size)

                    val editing = session.beginMetadata(FormKind.GROUP, group, null)
                    assertEquals(IconView.Lucide("server"), editing.icon)
                    val unfinished = session.patchGroupIcon(editing.form.draft, TextEdit.Set("lock"))
                    assertEquals(IconView.Lucide("lock"), unfinished.icon)
                    assertTrue(unfinished.form.dirty)
                    session.persistForms()
                    val path = manager.catalog().single().path
                    session.lock()
                    assertThrows(AndroidException::class.java) { session.metadata(unfinished.form.draft) }
                    manager.close(session)

                    val reopened = manager.open(path, password)
                    assertEquals(IconView.Lucide("server"), reopened.overview("").groups.single().icon)
                    assertEquals(1, reopened.history(FormKind.GROUP, group).size)
                    val continued = reopened.beginMetadata(FormKind.GROUP, group, null)
                    assertEquals(unfinished.form, continued.form)
                    assertEquals(IconView.Lucide("lock"), continued.icon)
                    val confirmed = reopened.save(continued.form.draft, continued.form.revision, newOperation())
                    assertEquals(SaveState.SAVED, confirmed.state)
                    reopened.deleteForm(confirmed.form.draft)
                    assertEquals(IconView.Lucide("lock"), reopened.overview("").groups.single().icon)
                    assertEquals(2, reopened.history(FormKind.GROUP, group).size)

                    val beforeInvalid = reopened.beginMetadata(FormKind.GROUP, group, null)
                    assertThrows(AndroidException.InvalidOptions::class.java) {
                        reopened.patchGroupIcon(beforeInvalid.form.draft, TextEdit.Set("PUBLIC invalid icon key"))
                    }
                    val unchanged = reopened.metadata(beforeInvalid.form.draft)
                    assertEquals(beforeInvalid.form, unchanged.form)
                    assertEquals(IconView.Lucide("lock"), unchanged.icon)
                    assertEquals(2, reopened.history(FormKind.GROUP, group).size)
                    val cleared = reopened.patchGroupIcon(unchanged.form.draft, TextEdit.Clear)
                    assertEquals(IconView.Default, cleared.icon)
                    assertEquals(IconView.Lucide("lock"), reopened.overview("").groups.single().icon)
                    val clearOperation = newOperation()
                    val clearedReceipt = reopened.save(cleared.form.draft, cleared.form.revision, clearOperation)
                    assertEquals(SaveState.SAVED, clearedReceipt.state)
                    assertEquals(clearedReceipt, reopened.save(cleared.form.draft, cleared.form.revision, clearOperation))
                    assertEquals(3, reopened.history(FormKind.GROUP, group).size)
                    reopened.deleteForm(clearedReceipt.form.draft)
                    reopened.lock()
                    manager.close(reopened)

                    val finalSession = manager.open(path, password)
                    assertEquals(IconView.Default, finalSession.overview("").groups.single().icon)
                    assertEquals(IconView.Default, finalSession.beginMetadata(FormKind.GROUP, group, null).icon)
                    assertEquals(3, finalSession.history(FormKind.GROUP, group).size)
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }
}
