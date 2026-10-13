package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.FormKind
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/** Real long touches open existing actions over a supervised encrypted document. */
class ContextActionsUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val state get() = document.state

    @Before fun createPublicDocument() {
        document = PublicDocumentUiFixture(compose)
        document.create()
    }

    @After fun restorePreferences() {
        if (::document.isInitialized) document.restorePreferences()
    }

    private fun longPressEntry(entry: String) {
        compose.onNodeWithTag("entry-$entry").performScrollTo().assertIsDisplayed().performTouchInput { longClick() }
        document.waitFor("entry-context-menu")
        compose.onNodeWithTag("entry-context-menu").assertIsDisplayed()
    }

    private fun longPressGroup(group: String) {
        compose.onNodeWithTag("group-menu").performClick()
        document.waitFor("database-drawer-open")
        compose.onNodeWithTag("select-group-$group").performScrollTo().assertIsDisplayed().performTouchInput { longClick() }
        document.waitFor("group-context-menu")
        document.waitFor("database-drawer-closed")
        compose.onNodeWithTag("group-context-menu").assertIsDisplayed()
    }

    private fun back() {
        // A sheet has its own dialog window; deliver Back to the actually focused OS window.
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
    }

    private fun unlock() {
        document.waitFor("master-password")
        compose.onNodeWithTag("master-password").performTextReplacement("PUBLIC lifecycle UI master password")
        compose.onNodeWithTag("authenticate").performClick()
        document.await { state.overview != null && !state.opening && !state.navigating }
    }

    private fun historyCount(kind: FormKind, target: String): Int {
        compose.runOnIdle { state.showHistory(kind, target) }
        document.await { state.history?.kind == kind && state.history?.target == target && !state.navigating }
        var count = 0
        compose.runOnIdle { count = state.history!!.rows.size; state.dismissHistory() }
        document.waitFor("group-menu")
        return count
    }

    @Test fun entryLongPressAndCancelAreReadOnlyAndCloneRequiresExplicitConfirmation() {
        val group = document.group("PUBLIC context source group")
        val destination = document.group("PUBLIC context clone destination")
        val fields = PublicEntryFields(title = "PUBLIC context original entry")
        val entry = document.entry(group, fields)
        assertEquals(1, historyCount(FormKind.ENTRY, entry))
        document.showGroup(group)
        val before = document.savedBytes()

        longPressEntry(entry)
        compose.runOnIdle {
            assertNull(state.inspectedEntry)
            assertNull(state.activeForm)
            assertTrue(state.revealed.isEmpty())
        }
        assertArrayEquals(before, document.savedBytes())
        back()
        document.waitAbsent("entry-context-menu")
        assertArrayEquals(before, document.savedBytes())
        assertEquals(1, historyCount(FormKind.ENTRY, entry))
        document.inspect(group, entry)
        compose.runOnIdle { assertNull(state.properties!!.accessedAt) }
        document.showGroup(group)

        longPressEntry(entry)
        compose.onNodeWithTag("context-clone-entry").performScrollTo().assertIsEnabled().performClick()
        document.waitFor("object-action-screen")
        document.waitAbsent("entry-context-menu")
        val name = "PUBLIC explicit context clone"
        compose.onNodeWithTag("clone-name").performTextReplacement(name)
        compose.onNodeWithTag("destination-$destination").performScrollTo().performClick()
        assertArrayEquals(before, document.savedBytes())
        compose.runOnIdle { assertEquals(1, state.overview!!.entries.size) }
        compose.onNodeWithTag("confirm-object-action").assertIsEnabled().performClick()
        document.waitAbsent("object-action-screen")
        document.await { state.overview?.entries?.size == 2 && !state.lifecycle.busy && !state.navigating }
        var clone = ""
        compose.runOnIdle {
            clone = state.overview!!.entries.single { it.id != entry }.id
            assertEquals(name, state.overview!!.entries.single { it.id == clone }.title)
            assertEquals(destination, state.overview!!.entries.single { it.id == clone }.group)
        }
        assertFalse(before.contentEquals(document.savedBytes()))
        assertEquals(1, historyCount(FormKind.ENTRY, entry))
        assertEquals(1, historyCount(FormKind.ENTRY, clone))
        document.reopen()
        document.inspect(group, entry)
        compose.runOnIdle { assertNull(state.properties!!.accessedAt) }
        document.inspect(destination, clone)
        compose.runOnIdle { assertEquals(fields.username, state.inspectedEntry!!.username); assertNull(state.properties!!.accessedAt) }
    }

    @Test fun groupLongPressDoesNotSelectOrMutateAndTrashUsesTheExistingExplicitAction() {
        val target = document.group("PUBLIC context group to trash")
        val selected = document.group("PUBLIC context selected group")
        val entry = document.entry(target, PublicEntryFields())
        document.showGroup(selected)
        val before = document.savedBytes()

        longPressGroup(target)
        compose.runOnIdle {
            assertEquals(selected, state.selectedGroup)
            assertNull(state.metadata)
            assertNull(state.activeForm)
        }
        assertArrayEquals(before, document.savedBytes())
        back()
        document.waitAbsent("group-context-menu")
        assertArrayEquals(before, document.savedBytes())
        assertEquals(1, historyCount(FormKind.GROUP, target))

        longPressGroup(target)
        compose.onNodeWithTag("trash-group").performScrollTo().assertIsEnabled().performClick()
        document.waitAbsent("group-context-menu")
        document.await { state.overview?.groups?.none { it.id == target } == true && !state.navigating }
        compose.runOnIdle {
            assertEquals(selected, state.selectedGroup)
            assertTrue(state.overview!!.groups.any { it.id == selected })
            assertTrue(state.overview!!.entries.none { it.id == entry })
            assertEquals(target, state.undo!!.target)
        }
        assertFalse(before.contentEquals(document.savedBytes()))
        document.reopen()
        document.menu("open-trash")
        document.waitFor("trash-screen")
        document.await { state.lifecycle.rows.any { it.kind == FormKind.GROUP && it.target == target } && !state.lifecycle.busy }
    }

    @Test fun contextEditKeepsOneDraftAndUsesStayDiscardAndSaveNavigation() {
        val group = document.group("PUBLIC context editor group")
        val fields = PublicEntryFields(title = "PUBLIC context edited entry")
        val entry = document.entry(group, fields)
        document.showGroup(group)
        val before = document.savedBytes()
        longPressEntry(entry)
        compose.onNodeWithTag("context-edit-entry").performScrollTo().assertIsEnabled().performClick()
        document.waitFor("entry-notes")
        document.await { state.entry?.source?.entry == entry && !state.editingBlocked }
        var draft = ""
        compose.runOnIdle { draft = state.activeForm!!.draft }
        val changed = "PUBLIC context unsaved notes"
        compose.onNodeWithTag("entry-notes").performScrollTo().performTextReplacement(changed)
        document.await { state.entry?.notes == changed && state.activeForm?.dirty == true && !state.editingBlocked }
        compose.onNodeWithTag("navigation-back").performClick()
        document.waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-stay").performClick()
        document.waitAbsent("unsaved-changes")
        compose.runOnIdle { assertEquals(draft, state.activeForm!!.draft); assertEquals(changed, state.entry!!.notes) }
        assertArrayEquals(before, document.savedBytes())
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        document.waitFor("master-password")
        compose.onNodeWithTag("entry-notes").assertDoesNotExist()
        unlock()
        document.waitFor("entry-notes")
        document.await { state.entry?.source?.entry == entry && !state.editingBlocked }
        compose.onNodeWithTag("entry-notes").performScrollTo().assertTextEquals(changed)
        compose.runOnIdle { assertEquals(draft, state.activeForm!!.draft) }
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("navigation-back").performClick()
        document.waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-discard").performClick()
        document.await { state.entry == null && state.activeForm == null && !state.navigating }
        document.waitFor("entry-$entry")
        assertArrayEquals(before, document.savedBytes())

        longPressEntry(entry)
        compose.onNodeWithTag("context-edit-entry").performScrollTo().performClick()
        document.await { state.entry?.source?.entry == entry && !state.editingBlocked }
        compose.onNodeWithTag("entry-notes").performScrollTo().assertTextEquals(fields.notes)
        compose.onNodeWithTag("entry-notes").performTextReplacement(changed)
        document.await { state.entry?.notes == changed && state.activeForm?.dirty == true && !state.editingBlocked }
        compose.onNodeWithTag("navigation-back").performClick()
        document.waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-save").performClick()
        document.waitAbsent("unsaved-changes")
        document.await { state.entry == null && state.activeForm == null && !state.navigating }
        assertEquals(2, historyCount(FormKind.ENTRY, entry))
        document.reopen()
        document.inspect(group, entry)
        compose.runOnIdle { assertEquals(changed, state.inspectedEntry!!.notes); assertNull(state.properties!!.accessedAt) }
    }

    @Test fun actualBackgroundClosesEntryAndGroupMenusWithoutChangingTheArchive() {
        val group = document.group("PUBLIC context background group")
        val entry = document.entry(group, PublicEntryFields())
        document.showGroup(group)
        val before = document.savedBytes()
        for (kind in listOf(FormKind.ENTRY, FormKind.GROUP)) {
            if (kind == FormKind.ENTRY) longPressEntry(entry) else longPressGroup(group)
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            document.waitFor("master-password")
            compose.onNodeWithTag("entry-context-menu").assertDoesNotExist()
            compose.onNodeWithTag("group-context-menu").assertDoesNotExist()
            compose.onNodeWithText(PublicEntryFields().title).assertDoesNotExist()
            compose.onNodeWithText("PUBLIC context background group").assertDoesNotExist()
            compose.runOnIdle { assertNull(state.overview); assertNull(state.entry); assertNull(state.metadata) }
            assertArrayEquals(before, document.savedBytes())
            unlock()
            document.waitFor("group-menu")
            compose.onNodeWithTag("entry-context-menu").assertDoesNotExist()
            compose.onNodeWithTag("group-context-menu").assertDoesNotExist()
            document.showGroup(group)
        }
        assertEquals(1, historyCount(FormKind.ENTRY, entry))
        assertEquals(1, historyCount(FormKind.GROUP, group))
    }
}
