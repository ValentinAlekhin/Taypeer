package dev.taypeer

import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.EntryEditor
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.SaveState
import dev.taypeer.bridge.TextEdit
import org.junit.Assert.assertEquals

/** PUBLIC fixtures use the same group form and explicit Save as the application. */
internal fun publicGroup(session: DocumentSession): String {
    session.overview("").groups.firstOrNull()?.let { return it.id }
    val initial = session.beginMetadata(FormKind.NEW_GROUP, null, null)
    val ready = session.patchMetadata(initial.form.draft, FormKind.NEW_GROUP,
        TextEdit.Set("PUBLIC fixture group"), TextEdit.Keep)
    assertEquals(SaveState.SAVED, session.save(ready.form.draft, ready.form.revision, newOperation()).state)
    session.deleteForm(ready.form.draft)
    return ready.form.target!!
}

internal fun beginPublicEntry(session: DocumentSession): EntryEditor = session.beginCreateEntry(publicGroup(session))
