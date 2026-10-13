package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.GroupRow
import dev.taypeer.bridge.IconView
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*

/** Actual selector handlers keep placement and metadata inside one supervised Rust form. */
class GroupParentUiTest {
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

    private fun openGroup(group: String, create: Boolean = false) {
        document.showGroup(group)
        compose.onNodeWithTag("group-menu").performClick()
        document.waitFor("database-drawer-open")
        compose.onNodeWithTag("edit-group-$group").performScrollTo().performClick()
        val action = if (create) "new-subgroup" else "edit-selected-group"
        document.waitFor(action)
        compose.onNodeWithTag(action).performClick()
        document.await { state.metadata != null && !state.editingBlocked }
        document.waitFor("metadata-parent")
    }

    private fun selectParent(group: String?) {
        compose.onNodeWithTag("metadata-parent").performScrollTo().assertIsEnabled().performClick()
        val action = group?.let { "metadata-parent-$it" } ?: "metadata-parent-root"
        document.waitFor(action)
        compose.onNodeWithTag(action).performScrollTo().performClick()
        document.await { state.metadata?.source?.parent == group && state.metadata?.parent == group && !state.editingBlocked }
    }

    private fun current(group: String): GroupRow {
        var row: GroupRow? = null
        compose.runOnIdle { row = state.overview!!.groups.single { it.id == group } }
        return row!!
    }

    private fun historyCount(group: String): Int {
        compose.runOnIdle { state.showHistory(FormKind.GROUP, group) }
        document.await { state.history?.target == group && !state.navigating }
        var size = 0
        compose.runOnIdle { size = state.history!!.rows.size; state.dismissHistory() }
        document.waitFor("group-menu")
        return size
    }

    private fun leaveAndDecide(action: String) {
        compose.onNodeWithTag("navigation-back").performClick()
        document.waitFor("unsaved-changes")
        compose.onNodeWithTag(action).performClick()
        document.waitAbsent("unsaved-changes")
        document.await { !state.navigating }
    }

    @Test fun newGroupChangesInitialParentInsideTheSameDraftAndOneSave() {
        val initialParent = document.group("PUBLIC initial parent")
        val chosenParent = document.group("PUBLIC chosen parent")
        val before = document.savedBytes()
        openGroup(initialParent, create = true)
        var draft = ""
        var target: String? = null
        compose.runOnIdle {
            draft = state.activeForm!!.draft; target = state.activeForm!!.target
            assertEquals(initialParent, state.metadata!!.parent)
        }
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC child with parent")
        compose.onNodeWithTag("metadata-description").performTextReplacement("PUBLIC child description")
        compose.runOnIdle { state.setGroupIcon("server") }
        selectParent(chosenParent)
        compose.runOnIdle {
            assertEquals(draft, state.activeForm!!.draft)
            assertEquals(target, state.activeForm!!.target)
            assertEquals("PUBLIC child with parent", state.metadata!!.source.name)
            assertEquals("PUBLIC child description", state.metadata!!.source.description)
            assertEquals(IconView.Lucide("server"), state.metadata!!.source.icon)
            assertTrue(state.overview!!.groups.none { it.name == "PUBLIC child with parent" })
        }
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("save-form").assertIsEnabled().performClick()
        document.await { state.activeForm == null && state.overview?.groups?.any { it.name == "PUBLIC child with parent" } == true }
        var group = ""
        compose.runOnIdle { group = state.overview!!.groups.single { it.name == "PUBLIC child with parent" }.id; assertEquals(group, state.selectedGroup) }
        assertEquals(chosenParent, current(group).parent)
        assertEquals("PUBLIC child description", current(group).description)
        assertEquals(IconView.Lucide("server"), current(group).icon)
        assertFalse(before.contentEquals(document.savedBytes()))
        assertEquals(1, historyCount(group))
        document.reopen()
        assertEquals(chosenParent, current(group).parent)
        assertEquals(1, historyCount(group))
    }

    @Test fun existingGroupParentUsesStayDiscardAndNavigationSaveWithoutPartialWrites() {
        val group = document.group("PUBLIC existing group")
        val parent = document.group("PUBLIC navigation parent")
        assertEquals(1, historyCount(group))
        val before = document.savedBytes()
        openGroup(group)
        var draft = ""
        compose.runOnIdle {
            draft = state.activeForm!!.draft
            assertTrue(state.metadata!!.source.parentCandidates.none { it.id == group })
        }
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC discarded group")
        selectParent(parent)
        leaveAndDecide("navigation-stay")
        compose.runOnIdle {
            assertEquals(draft, state.activeForm!!.draft)
            assertEquals(parent, state.metadata!!.parent)
            assertEquals("PUBLIC discarded group", state.metadata!!.name)
        }
        assertNull(current(group).parent)
        assertArrayEquals(before, document.savedBytes())
        leaveAndDecide("navigation-discard")
        document.await { state.activeForm == null }
        assertEquals("PUBLIC existing group", current(group).name)
        assertNull(current(group).parent)
        assertArrayEquals(before, document.savedBytes())
        assertEquals(1, historyCount(group))

        openGroup(group)
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC saved group")
        selectParent(parent)
        leaveAndDecide("navigation-save")
        document.await { state.activeForm == null }
        assertEquals("PUBLIC saved group", current(group).name)
        assertEquals(parent, current(group).parent)
        assertEquals(2, historyCount(group))
        openGroup(group)
        selectParent(null)
        compose.onNodeWithTag("save-form").performClick()
        document.await { state.activeForm == null }
        assertNull(current(group).parent)
        assertEquals(3, historyCount(group))
    }

    @Test fun newerParentInputAfterSaveRemainsInTheSameDirtyForm() {
        val group = document.group("PUBLIC queued group")
        val parent = document.group("PUBLIC queued parent")
        openGroup(group)
        selectParent(parent)
        var draft = ""
        compose.runOnIdle {
            draft = state.activeForm!!.draft
            // Real queue ordering: the Save snapshot precedes the new parent patch.
            state.saveNow()
            state.editGroupParent(null)
        }
        document.await { state.activeForm?.draft == draft && state.saveLabel == SaveLabel.DIRTY && state.metadata?.source?.parent == null && state.overview?.groups?.single { it.id == group }?.parent == parent }
        compose.runOnIdle { assertNull(state.metadata!!.parent); assertTrue(state.activeForm!!.dirty) }
        val afterFirstSave = document.savedBytes()
        assertEquals(parent, current(group).parent)
        compose.onNodeWithTag("metadata-parent").assertTextContains(document.text(R.string.group_parent_root))
        assertArrayEquals(afterFirstSave, document.savedBytes())
        compose.onNodeWithTag("save-form").performClick()
        document.await { state.activeForm == null }
        assertNull(current(group).parent)
        assertFalse(afterFirstSave.contentEquals(document.savedBytes()))
        assertEquals(3, historyCount(group))
    }

    @Test fun invalidParentRetainsInputsUntilCorrectedSave() {
        val group = document.group("PUBLIC rejected group")
        assertEquals(1, historyCount(group))
        val before = document.savedBytes()
        openGroup(group)
        var draft = ""
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC corrected group")
        compose.onNodeWithTag("metadata-description").performTextReplacement("PUBLIC retained description")
        compose.runOnIdle {
            draft = state.activeForm!!.draft
            state.editGroupParent("PUBLIC missing destination")
        }
        document.await { state.invalidInput && state.saveLabel == SaveLabel.FAILED }
        compose.runOnIdle {
            assertEquals(draft, state.activeForm!!.draft)
            assertEquals("PUBLIC missing destination", state.metadata!!.parent)
            assertEquals("PUBLIC corrected group", state.metadata!!.name)
            assertEquals("PUBLIC retained description", state.metadata!!.description)
        }
        assertEquals("PUBLIC rejected group", current(group).name)
        assertArrayEquals(before, document.savedBytes())
        selectParent(null)
        compose.onNodeWithTag("save-form").performClick()
        document.await { state.activeForm == null }
        assertEquals("PUBLIC corrected group", current(group).name)
        assertEquals("PUBLIC retained description", current(group).description)
        assertNull(current(group).parent)
        assertEquals(2, historyCount(group))
    }
}
