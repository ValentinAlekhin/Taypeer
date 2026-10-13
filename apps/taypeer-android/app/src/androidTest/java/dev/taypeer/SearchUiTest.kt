package dev.taypeer

import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.semantics.SemanticsActions
import androidx.lifecycle.Lifecycle
import dev.taypeer.bridge.*
import dev.taypeer.platform.OpenDocumentSource
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import java.io.File

/** Search gestures exercise two real isolated workers and the existing Rust matching predicate. */
class SearchUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val state get() = document.state
    private val password = "PUBLIC lifecycle UI master password"
    private data class SavedSource(val copy: WorkingCopyView, val group: String, val entry: String,
        val generation: ULong, val bytes: ByteArray, val parent: String? = null)

    @Before fun createPublicDocument() { document = PublicDocumentUiFixture(compose); document.create() }
    @After fun restorePreferences() { if (::document.isInitialized) document.restorePreferences() }

    private fun firstSource(protected: Boolean = false): SavedSource {
        val group = document.group("PUBLIC search first group")
        val entry = document.entry(group, PublicEntryFields(title = "PUBLIC studio first", password = "PUBLIC password-only needle"))
        if (protected) {
            compose.runOnIdle { state.activity(); state.beginEntry(entry) }
            document.await { state.entry?.source?.entry == entry && !state.editingBlocked }
            compose.runOnIdle { state.editAttribute(null, "PUBLIC hidden attribute", "PUBLIC protected-only needle", true) }
            document.await { state.entry?.source?.attributes?.any { it.protected } == true }
            compose.runOnIdle { state.saveNow() }
            document.await { state.activeForm == null && state.inspectedEntry?.id == entry && !state.navigating }
        }
        return capture(group, entry)
    }
    private fun secondSource(): SavedSource {
        compose.runOnIdle { state.activity(); state.create("PUBLIC search second database", null, password, newOperation()) }
        document.await { state.overview?.name == "PUBLIC search second database" && !state.opening && !state.navigating }
        val parent = document.group("PUBLIC search second root group")
        compose.runOnIdle { state.activity(); state.beginMetadata(FormKind.NEW_GROUP, parent = parent) }
        document.await { state.metadata != null && !state.navigating }
        compose.runOnIdle { state.editMetadata(name = "PUBLIC search second group"); state.saveNow() }
        document.await { state.activeForm == null && state.overview?.groups?.any { it.name == "PUBLIC search second group" } == true && !state.navigating }
        var group = ""
        compose.runOnIdle { group = state.overview!!.groups.single { it.name == "PUBLIC search second group" }.id }
        val entry = document.entry(group, PublicEntryFields(title = "PUBLIC studio second", username = "PUBLIC ordinary search marker"))
        return capture(group, entry).copy(parent = parent)
    }
    private fun capture(group: String, entry: String): SavedSource {
        lateinit var copy: WorkingCopyView
        var generation = 0uL
        compose.runOnIdle {
            copy = state.catalog.single { it.database == state.overview!!.database }
            generation = state.overview!!.generation
        }
        return SavedSource(copy, group, entry, generation, File(copy.path).readBytes())
    }
    private fun unchanged(vararg sources: SavedSource) = sources.forEach { assertArrayEquals(it.bytes, File(it.copy.path).readBytes()) }
    private fun tap(tag: String) {
        compose.runOnIdle {
            compose.activity.getSystemService(InputMethodManager::class.java)
                .hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0)
        }
        compose.waitUntil(10_000) { compose.activity.window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.ime()) == false }
        val node = compose.onNodeWithTag(tag)
        val ancestors = generateSequence(node.fetchSemanticsNode().parent) { it.parent }
        if (ancestors.any { SemanticsActions.ScrollBy in it.config }) node.performScrollTo()
        // Scroll/layout can replace the semantics node even when its target
        // remains the same. Resolve the current row before the physical tap.
        compose.onNodeWithTag(tag).assertIsDisplayed().assertIsEnabled().performTouchInput { click() }
    }
    private fun search(query: String, all: Boolean = false) {
        if (state.inspectedEntry != null) tap("navigation-back")
        document.waitFor("entry-search")
        compose.onNodeWithTag("entry-search").performTextReplacement(query)
        document.await { state.searches.query == query && state.searches.showing && !state.searches.busy }
        if (all) {
            tap("search-scope"); document.waitFor("search-scope-sheet")
            tap("search-all-unlocked"); document.waitAbsent("search-scope-sheet")
            document.await { state.searches.scope == SearchScope.ALL_UNLOCKED && !state.searches.busy }
        }
    }
    private fun rows(): List<SearchResultRow> { var result = emptyList<SearchResultRow>(); compose.runOnIdle { result = state.searches.results }; return result }
    private fun source(row: SearchResultRow): OpenDocumentSource {
        var result: OpenDocumentSource? = null
        compose.runOnIdle { result = state.searches.source(row) }
        return requireNotNull(result)
    }
    private fun select(saved: SavedSource) {
        tap("search-entry-${saved.copy.database}-${saved.entry}")
        document.await { state.overview?.database == saved.copy.database && state.inspectedEntry?.id == saved.entry && !state.opening && !state.navigating }
        compose.runOnIdle { assertEquals(saved.group, state.selectedGroup); assertEquals(saved.generation, state.overview!!.generation) }
    }

    @Test fun scopeUsesActualOpenWorkersAndRustExcludesPasswordsAndProtectedValues() {
        val first = firstSource(protected = true)
        val second = secondSource()
        document.showGroup(requireNotNull(second.parent))
        search("PUBLIC studio")
        assertEquals(listOf(second.entry), rows().map { it.entry.id })
        compose.runOnIdle {
            state.searches.refresh()
            assertTrue(state.searches.busy)
            assertEquals(listOf(second.entry), state.searches.results.map { it.entry.id })
        }
        document.await { !state.searches.busy && rows().singleOrNull()?.entry?.id == second.entry }
        tap("search-scope"); tap("search-all-unlocked")
        document.await { rows().size == 2 && !state.searches.busy }
        val all = rows()
        assertEquals(setOf(first.copy.database, second.copy.database), all.map { it.copy.database }.toSet())
        assertEquals(setOf(first.group, second.group), all.map { it.entry.group }.toSet())
        all.forEach { row ->
            val native = source(row).session
            assertNull(native.entryProperties(row.entry.id).accessedAt)
            assertEquals(if (row.copy.database == first.copy.database) 2 else 1, native.history(FormKind.ENTRY, row.entry.id).size)
            compose.onNodeWithText("${row.databaseName} / ${row.groupName}", substring = true).assertExists()
        }
        for (excluded in listOf("PUBLIC password-only needle", "PUBLIC protected-only needle")) {
            compose.onNodeWithTag("entry-search").performTextReplacement(excluded)
            document.await { state.searches.query == excluded && !state.searches.busy }
            assertTrue(rows().isEmpty())
        }
        compose.onNodeWithTag("entry-search").performTextReplacement("PUBLIC ordinary search marker")
        document.await { !state.searches.busy && rows().singleOrNull()?.entry?.id == second.entry }
        unchanged(first, second)
        tap("clear-search-header")
        document.await { !state.searches.showing && state.query.isEmpty() }
        unchanged(first, second)
    }

    @Test fun crossDatabaseResultAndReturnKeepExactSourceGenerationQueryAndScope() {
        val first = firstSource()
        val second = secondSource()
        document.showGroup(second.group)
        search("PUBLIC studio", all = true)
        document.await { rows().size == 2 }
        val originalWorker = source(rows().single { it.entry.id == first.entry })
        select(first)
        compose.runOnIdle {
            assertNull(state.entry); assertNull(state.activeForm)
            assertEquals(2, state.unlockedCopies.size)
            assertEquals(SearchScope.ALL_UNLOCKED, state.searches.returning!!.scope)
            assertEquals(second.copy.database, state.searches.returning!!.copy.database)
            assertEquals("PUBLIC studio", state.searches.returning!!.query)
            assertNull(state.properties!!.accessedAt)
        }
        assertEquals(1, originalWorker.session.history(FormKind.ENTRY, first.entry).size)
        tap("return-search-results")
        document.await { state.overview?.database == second.copy.database && state.searches.showing && !state.searches.busy && rows().size == 2 }
        compose.runOnIdle { assertEquals(second.group, state.selectedGroup); assertEquals(second.generation, state.overview!!.generation) }
        assertSame(originalWorker, source(rows().single { it.entry.id == first.entry }))
        unchanged(first, second)
    }

    @Test fun returningFromCrossDatabaseEditRetainsStayDiscardAndOneSave() {
        val first = firstSource()
        val second = secondSource()
        document.showGroup(second.group)
        search("PUBLIC studio", all = true)
        document.await { rows().size == 2 }
        val firstWorker = source(rows().single { it.entry.id == first.entry })
        select(first)
        tap("edit-entry")
        document.await { state.entry?.source?.entry == first.entry && !state.editingBlocked }
        val notes = "PUBLIC search navigation unsaved notes"
        compose.onNodeWithTag("entry-notes").performScrollTo().performTextReplacement(notes)
        document.await { state.activeForm?.dirty == true && !state.editingBlocked }
        var draft = ""
        compose.runOnIdle { draft = state.activeForm!!.draft }
        tap("return-search-results"); document.waitFor("unsaved-changes")
        tap("navigation-stay"); document.waitAbsent("unsaved-changes")
        compose.runOnIdle {
            assertEquals(first.copy.database, state.overview!!.database)
            assertEquals(draft, state.activeForm!!.draft); assertEquals(notes, state.entry!!.notes)
            assertFalse(state.searches.showing)
        }
        unchanged(first, second)
        tap("return-search-results"); document.waitFor("unsaved-changes")
        tap("navigation-discard")
        document.await { state.overview?.database == second.copy.database && state.searches.showing && !state.searches.busy && rows().size == 2 }
        unchanged(first, second)
        assertEquals(1, firstWorker.session.history(FormKind.ENTRY, first.entry).size)
        select(first); tap("edit-entry")
        document.await { state.entry?.source?.entry == first.entry && !state.editingBlocked }
        compose.onNodeWithTag("entry-notes").performScrollTo().performTextReplacement(notes)
        document.await { state.activeForm?.dirty == true && !state.editingBlocked }
        tap("return-search-results"); document.waitFor("unsaved-changes"); tap("navigation-save")
        document.await { state.overview?.database == second.copy.database && state.searches.showing && !state.searches.busy && rows().size == 2 }
        assertEquals(notes, firstWorker.session.entry(first.entry).notes)
        assertEquals(2, firstWorker.session.history(FormKind.ENTRY, first.entry).size)
        assertFalse(first.bytes.contentEquals(File(first.copy.path).readBytes()))
        unchanged(second)
        assertNull(firstWorker.session.entryProperties(first.entry).accessedAt)
    }

    @Test fun manualLockKeepsOtherWorkerAndActualBackgroundRevokesEverySearchSource() {
        val first = firstSource()
        val second = secondSource()
        document.showGroup(second.group)
        search("PUBLIC studio", all = true)
        document.await { rows().size == 2 }
        val firstWorker = source(rows().single { it.entry.id == first.entry })
        val secondWorker = source(rows().single { it.entry.id == second.entry })
        select(second)
        document.menu("lock-database")
        document.waitFor("master-password")
        document.await { !secondWorker.permitted() && state.unlockedCopies.size == 1 }
        assertTrue(firstWorker.permitted())
        unchanged(first, second)
        tap("choose-another-database")
        document.waitFor("catalog-${first.copy.database}")
        tap("catalog-${first.copy.database}")
        document.await { state.overview?.database == first.copy.database && !state.opening && !state.navigating }
        compose.onNodeWithTag("master-password").assertDoesNotExist()
        compose.runOnIdle { assertEquals(first.generation, state.overview!!.generation) }
        search("PUBLIC studio", all = true)
        document.await { !state.searches.busy && rows().singleOrNull()?.entry?.id == first.entry }
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        document.waitFor("master-password")
        document.await { !firstWorker.permitted() && state.unlockedCopies.isEmpty() && state.searches.results.isEmpty() && !state.searches.showing }
        compose.onNodeWithTag("search-results").assertDoesNotExist()
        compose.runOnIdle { assertNull(state.overview); assertNull(state.inspectedEntry); assertNull(state.properties) }
        unchanged(first, second)
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.tapWorkingCopyUnlock(first.copy.database)
        document.await { state.overview?.database == first.copy.database && !state.opening }
        document.inspect(first.group, first.entry)
        compose.runOnIdle { assertNull(state.properties!!.accessedAt) }
        assertEquals(1, firstWorkerHistoryAfterReopen(first.entry))
        unchanged(first, second)
    }
    private fun firstWorkerHistoryAfterReopen(entry: String): Int {
        compose.runOnIdle { state.showHistory(FormKind.ENTRY, entry) }
        document.await { state.history?.target == entry && !state.navigating }
        var size = 0
        compose.runOnIdle { size = state.history!!.rows.size; state.dismissHistory() }
        return size
    }
}
