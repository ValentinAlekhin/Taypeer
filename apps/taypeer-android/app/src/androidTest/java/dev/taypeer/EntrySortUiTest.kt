package dev.taypeer

import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import dev.taypeer.bridge.FormKind
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import java.io.File

/** Sorting is a UI action over actual masked worker rows, without any archive write. */
class EntrySortUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val state get() = document.state
    @Before fun createPublicDocument() { document = PublicDocumentUiFixture(compose); document.create() }
    @After fun restorePreferences() { if (::document.isInitialized) document.restorePreferences() }

    private fun tap(tag: String) {
        compose.runOnIdle {
            state.activity()
            compose.activity.getSystemService(InputMethodManager::class.java)
                .hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0)
        }
        compose.waitUntil(10_000) { compose.activity.window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.ime()) == false }
        val ancestors = generateSequence(compose.onNodeWithTag(tag).fetchSemanticsNode().parent) { it.parent }
        if (ancestors.any { SemanticsActions.ScrollBy in it.config }) compose.onNodeWithTag(tag).performScrollTo()
        compose.onNodeWithTag(tag).assertIsDisplayed().assertIsEnabled().performTouchInput { click() }
    }
    private fun sort(value: EntrySort, search: Boolean = false) {
        if (search) tap("sort-search") else { tap("screen-actions"); tap("sort-entries") }
        document.waitFor("entry-sort-sheet")
        tap("sort-${value.name.lowercase(java.util.Locale.ROOT)}")
        document.waitAbsent("entry-sort-sheet")
        compose.runOnIdle { assertEquals(value, state.entrySort) }
    }
    private fun ordinaryOrder(ids: List<String>): List<String> = ids.map { id ->
        id to compose.onNodeWithTag("entry-$id").fetchSemanticsNode().positionInRoot.y
    }.sortedBy { it.second }.map { it.first }
    private fun searchRows(): List<SearchResultRow> {
        var rows = emptyList<SearchResultRow>()
        compose.runOnIdle { rows = state.searches.results }
        return rows
    }

    @Test fun fourChoicesKeepSelectedIdHistoryAndBytesAcrossRecreationAndLock() {
        val group = document.group("PUBLIC sort group")
        val bravo = document.entry(group, PublicEntryFields(title = "PUBLIC bravo", username = "z-login"))
        val alpha = document.entry(group, PublicEntryFields(title = "PUBLIC alpha", username = "a-login"))
        val charlie = document.entry(group, PublicEntryFields(title = "PUBLIC charlie", username = ""))
        val ids = listOf(bravo, alpha, charlie)
        val before = document.savedBytes()
        document.showGroup(group)
        assertEquals(listOf(alpha, bravo, charlie), ordinaryOrder(ids))
        sort(EntrySort.TITLE_DESC)
        assertEquals(listOf(charlie, bravo, alpha), ordinaryOrder(ids))
        sort(EntrySort.USERNAME_ASC)
        assertEquals(listOf(alpha, bravo, charlie), ordinaryOrder(ids))
        sort(EntrySort.MODIFIED_NEWEST)
        assertEquals(listOf(charlie, alpha, bravo), ordinaryOrder(ids))

        document.inspect(group, bravo)
        sort(EntrySort.TITLE_ASC)
        compose.runOnIdle { assertEquals(bravo, state.inspectedEntry?.id) }
        assertArrayEquals(before, document.savedBytes())
        document.showGroup(group)
        sort(EntrySort.USERNAME_ASC)
        compose.activityRule.scenario.recreate()
        document.waitFor("group-menu")
        compose.runOnIdle { assertEquals(EntrySort.USERNAME_ASC, state.entrySort) }
        assertEquals(listOf(alpha, bravo, charlie), ordinaryOrder(ids))
        document.reopen()
        compose.runOnIdle { assertEquals(EntrySort.USERNAME_ASC, state.entrySort) }
        assertEquals(listOf(alpha, bravo, charlie), ordinaryOrder(ids))
        assertArrayEquals(before, document.savedBytes())
        document.inspect(group, bravo)
        compose.runOnIdle { state.showHistory(FormKind.ENTRY, bravo) }
        document.await { state.history?.target == bravo && !state.navigating }
        compose.runOnIdle { assertEquals(1, state.history!!.rows.size) }
        assertArrayEquals(before, document.savedBytes())
    }

    @Test fun allUnlockedSearchUsesSameOrderAndKeepsExactSourceWithoutSaving() {
        val firstGroup = document.group("PUBLIC first sort group")
        val firstAlpha = document.entry(firstGroup, PublicEntryFields(title = "PUBLIC sort alpha", username = "b-login"))
        val firstZeta = document.entry(firstGroup, PublicEntryFields(title = "PUBLIC sort zeta", username = ""))
        var firstDatabase = ""
        var firstPath = ""
        compose.runOnIdle {
            firstDatabase = state.overview!!.database
            firstPath = state.catalog.single { it.database == firstDatabase }.path
            state.activity()
            state.create("PUBLIC second sort database", null, "PUBLIC lifecycle UI master password", newOperation())
        }
        val firstBytes = File(firstPath).readBytes()
        document.await { state.overview?.name == "PUBLIC second sort database" && !state.opening && !state.navigating }
        val secondGroup = document.group("PUBLIC second sort group")
        val secondAlpha = document.entry(secondGroup, PublicEntryFields(title = "PUBLIC sort alpha", username = "a-login"))
        val secondBeta = document.entry(secondGroup, PublicEntryFields(title = "PUBLIC sort beta", username = "b-login"))
        var secondDatabase = ""
        var secondPath = ""
        compose.runOnIdle {
            secondDatabase = state.overview!!.database
            secondPath = state.catalog.single { it.database == secondDatabase }.path
        }
        val secondBytes = File(secondPath).readBytes()
        document.showGroup(secondGroup)
        compose.onNodeWithTag("entry-search").performTextReplacement("PUBLIC sort")
        document.await { state.searches.showing && state.searches.query == "PUBLIC sort" && state.searches.results.size == 2 && !state.searches.busy }
        tap("search-scope"); document.waitFor("search-scope-sheet"); tap("search-all-unlocked")
        document.await { state.searches.scope == SearchScope.ALL_UNLOCKED && state.searches.results.size == 4 && !state.searches.busy }
        val alphaTies = listOf(firstDatabase to firstAlpha, secondDatabase to secondAlpha).sortedBy { it.first }
        assertEquals(alphaTies + listOf(secondDatabase to secondBeta, firstDatabase to firstZeta),
            searchRows().map { it.copy.database to it.entry.id })
        sort(EntrySort.TITLE_DESC, search = true)
        assertEquals(listOf(firstDatabase to firstZeta, secondDatabase to secondBeta) + alphaTies,
            searchRows().map { it.copy.database to it.entry.id })
        sort(EntrySort.USERNAME_ASC, search = true)
        val loginTies = listOf(firstDatabase to firstAlpha, secondDatabase to secondBeta).sortedBy { it.first }
        assertEquals(listOf(secondDatabase to secondAlpha) + loginTies + (firstDatabase to firstZeta),
            searchRows().map { it.copy.database to it.entry.id })
        sort(EntrySort.MODIFIED_NEWEST, search = true)
        assertEquals(listOf(secondBeta, secondAlpha, firstZeta, firstAlpha), searchRows().map { it.entry.id })
        searchRows().forEach { row ->
            val source = requireNotNull(state.searches.source(row))
            assertEquals(row.entry.modifiedAt, source.session.entryProperties(row.entry.id).modifiedAt)
            assertEquals(row.generation, source.session.generation())
        }
        tap("search-entry-$firstDatabase-$firstAlpha")
        document.await { state.overview?.database == firstDatabase && state.inspectedEntry?.id == firstAlpha && !state.navigating }
        compose.runOnIdle { assertEquals(firstGroup, state.selectedGroup) }
        tap("navigation-back")
        document.await { state.searches.showing && state.searches.results.size == 4 && !state.searches.busy }
        compose.runOnIdle { assertEquals(EntrySort.MODIFIED_NEWEST, state.entrySort) }
        assertEquals(listOf(secondBeta, secondAlpha, firstZeta, firstAlpha), searchRows().map { it.entry.id })
        assertArrayEquals(firstBytes, File(firstPath).readBytes())
        assertArrayEquals(secondBytes, File(secondPath).readBytes())
    }
}
