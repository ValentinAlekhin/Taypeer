package dev.taypeer

import dev.taypeer.bridge.EntryRow
import dev.taypeer.bridge.IconView
import org.junit.Assert.assertEquals
import org.junit.Test
import java.util.Locale

class EntrySortTest {
    private data class Row(val database: String, val entry: EntryRow)
    private fun row(database: String, id: String, title: String, username: String? = null, modified: Long = 0) =
        Row(database, EntryRow(id, "PUBLIC-group", title, username, null, null, null, false, IconView.Default, modified))
    private fun sorted(rows: List<Row>, value: EntrySort) =
        rows.sortedWith(value.comparator({ it.database }, { it.entry })).map { it.database to it.entry.id }

    @Test fun titleDirectionsUseStableSourceAndEntryTiesWithoutLocaleOrInputMutation() {
        val rows = listOf(row("PUBLIC-b", "PUBLIC-1", "alpha"), row("PUBLIC-a", "PUBLIC-2", "ALPHA"),
            row("PUBLIC-a", "PUBLIC-1", "Alpha"), row("PUBLIC-a", "PUBLIC-3", "Beta"))
        val original = rows.map { it.database to it.entry.id }
        val locale = Locale.getDefault()
        try {
            Locale.setDefault(Locale.forLanguageTag("tr-TR"))
            assertEquals(listOf("PUBLIC-a" to "PUBLIC-1", "PUBLIC-a" to "PUBLIC-2", "PUBLIC-b" to "PUBLIC-1",
                "PUBLIC-a" to "PUBLIC-3"), sorted(rows, EntrySort.TITLE_ASC))
            assertEquals(listOf("PUBLIC-a" to "PUBLIC-3", "PUBLIC-a" to "PUBLIC-1", "PUBLIC-a" to "PUBLIC-2",
                "PUBLIC-b" to "PUBLIC-1"), sorted(rows, EntrySort.TITLE_DESC))
            assertEquals(original, rows.map { it.database to it.entry.id })
        } finally { Locale.setDefault(locale) }
    }

    @Test fun usernamesPutAbsentAndEmptyLastWithStableSourceIdentity() {
        val rows = listOf(row("PUBLIC-a", "PUBLIC-null", "Z", null), row("PUBLIC-b", "PUBLIC-empty", "A", ""),
            row("PUBLIC-b", "PUBLIC-alpha", "Z", "alpha"), row("PUBLIC-a", "PUBLIC-alpha", "A", "ALPHA"),
            row("PUBLIC-a", "PUBLIC-beta", "B", "beta"))
        assertEquals(listOf("PUBLIC-a" to "PUBLIC-alpha", "PUBLIC-b" to "PUBLIC-alpha", "PUBLIC-a" to "PUBLIC-beta",
            "PUBLIC-a" to "PUBLIC-null", "PUBLIC-b" to "PUBLIC-empty"), sorted(rows, EntrySort.USERNAME_ASC))
    }

    @Test fun newestUsesActualMillisecondsWithoutOverflowAndKeepsEqualTimeIdentityStable() {
        val rows = listOf(row("PUBLIC-a", "PUBLIC-old", "A", modified = Long.MIN_VALUE),
            row("PUBLIC-b", "PUBLIC-same", "B", modified = 1), row("PUBLIC-a", "PUBLIC-same", "Z", modified = 1),
            row("PUBLIC-a", "PUBLIC-new", "A", modified = Long.MAX_VALUE))
        assertEquals(listOf("PUBLIC-a" to "PUBLIC-new", "PUBLIC-a" to "PUBLIC-same", "PUBLIC-b" to "PUBLIC-same",
            "PUBLIC-a" to "PUBLIC-old"), sorted(rows, EntrySort.MODIFIED_NEWEST))
    }
}
