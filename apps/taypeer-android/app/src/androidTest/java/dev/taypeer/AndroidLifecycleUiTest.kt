package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/** Verifies Compose lifecycle wiring over native durable operations, rather than reimplementing it. */
class AndroidLifecycleUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val current = PublicEntryFields()

    @Before fun createPublicDocument() {
        document = PublicDocumentUiFixture(compose)
        document.create()
    }

    @After fun restorePreferences() {
        if (::document.isInitialized) document.restorePreferences()
    }

    private fun assertCurrentFields(entry: String, fields: PublicEntryFields = current) {
        compose.onNodeWithTag("entry-tab-0").performScrollTo().performClick()
        document.await { document.state.entryTab == 0 && !document.state.navigating }
        compose.runOnIdle {
            val shown = document.state.inspectedEntry!!
            assertEquals(entry, shown.id)
            assertEquals(fields.title, shown.title)
            assertEquals(fields.username, shown.username)
            assertEquals(fields.url, shown.url)
            assertEquals(fields.notes, shown.notes)
            assertEquals(fields.tags.toSet(), document.state.properties!!.tags.toSet())
        }
        compose.onNodeWithText(fields.username).assertExists()
        compose.onNodeWithTag("reveal-password").performScrollTo().performClick()
        document.await { document.state.revealed == fields.password }
        compose.onNodeWithText(fields.password).assertExists()
    }

    private fun openTrash() {
        document.waitFor("group-menu")
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithTag("drawer-trash").performScrollTo().performClick()
        document.waitFor("trash-screen")
        document.await { document.state.lifecycle.showingTrash && !document.state.lifecycle.busy }
    }

    private fun trashEntry() {
        document.menu("trash-entry")
        document.waitFor("group-menu")
        document.await { document.state.inspectedEntry == null && !document.state.navigating }
    }

    private fun prepareRestore(entry: String, destination: String) {
        compose.onNodeWithTag("trash-$entry").performScrollTo().performClick()
        compose.onNodeWithTag("choose-trash-restore").performScrollTo().performClick()
        compose.onNodeWithTag("destination-$destination").performScrollTo().performClick()
        compose.onNodeWithTag("prepare-trash-restore").assertIsEnabled().performClick()
        waitForReview(entry, restoring = true)
    }

    private fun preparePurge(entry: String) {
        compose.onNodeWithTag("trash-$entry").performScrollTo().performClick()
        compose.onNodeWithTag("prepare-trash-purge").performScrollTo().performClick()
        waitForReview(entry, restoring = false)
    }

    private fun waitForReview(entry: String, restoring: Boolean) {
        document.waitFor("trash-review")
        document.await { document.state.lifecycle.review != null && !document.state.lifecycle.busy }
        compose.runOnIdle {
            val reviewed = document.state.lifecycle.review!!
            assertEquals(entry, reviewed.row.target)
            assertEquals(restoring, reviewed.restoring)
            assertEquals(0u, reviewed.review.groups)
            assertEquals(1u, reviewed.review.entries)
        }
        compose.onNodeWithText(document.text(R.string.trash_review_counts, "0", "1")).assertExists()
    }

    private fun cancelReview() {
        compose.onNodeWithTag("cancel-trash-review").assertIsEnabled().performClick()
        document.waitAbsent("trash-review")
        document.await { document.state.lifecycle.review == null && !document.state.lifecycle.busy }
    }

    @Test fun cloneCopiesCurrentFieldsToANewIdentityWithoutPreviousHistory() {
        val source = document.group("PUBLIC lifecycle clone source")
        val destination = document.group("PUBLIC lifecycle clone destination")
        val previous = current.copy(title = "PUBLIC lifecycle previous entry", password = "PUBLIC lifecycle previous password", username = "PUBLIC previous username")
        val original = document.entry(source, previous)
        document.update(original, current)
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == 2 }
        compose.onNodeWithTag("entry-tab-0").performScrollTo().performClick()
        val before = document.savedBytes()
        val cloneTitle = "PUBLIC lifecycle cloned current entry"

        document.menu("clone-entry")
        document.waitFor("object-action-screen")
        compose.onNodeWithTag("clone-name").performTextReplacement(cloneTitle)
        compose.onNodeWithTag("destination-$destination").performScrollTo().performClick()
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("confirm-object-action").assertIsEnabled().performClick()
        document.waitAbsent("object-action-screen")
        document.await { document.state.overview?.entries?.size == 2 && !document.state.lifecycle.busy }
        var clone = ""
        compose.runOnIdle {
            val rows = document.state.overview!!.entries
            assertTrue(rows.any { it.id == original && it.group == source && it.title == current.title })
            clone = rows.single { it.id != original }.id
            assertEquals(destination, rows.single { it.id == clone }.group)
            assertEquals(cloneTitle, rows.single { it.id == clone }.title)
        }
        assertNotEquals(original, clone)
        document.inspect(destination, clone)
        assertCurrentFields(clone, current.copy(title = cloneTitle))
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == 1 }
        compose.runOnIdle { assertEquals(cloneTitle, document.state.entryHistory.single().name) }
        compose.onNodeWithText(previous.title).assertDoesNotExist()

        document.reopen()
        document.inspect(destination, clone)
        assertCurrentFields(clone, current.copy(title = cloneTitle))
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == 1 }
        compose.runOnIdle { assertEquals(2, document.state.overview!!.entries.size) }
    }

    @Test fun moveKeepsTheEntryIdentityAndFieldsInTheChosenGroupAfterReopen() {
        val source = document.group("PUBLIC lifecycle move source")
        val destination = document.group("PUBLIC lifecycle move destination")
        val original = document.entry(source, current)
        val before = document.savedBytes()

        document.menu("move-entry")
        document.waitFor("object-action-screen")
        compose.onNodeWithTag("clone-name").assertDoesNotExist()
        compose.onNodeWithTag("destination-$destination").performScrollTo().performClick()
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("confirm-object-action").assertIsEnabled().performClick()
        document.waitAbsent("object-action-screen")
        document.await { document.state.overview?.entries?.singleOrNull()?.group == destination && !document.state.lifecycle.busy }
        compose.runOnIdle { assertEquals(original, document.state.overview!!.entries.single().id) }

        document.reopen()
        document.showGroup(source)
        compose.onNodeWithTag("entry-$original").assertDoesNotExist()
        document.inspect(destination, original)
        assertCurrentFields(original)
        compose.runOnIdle { assertEquals(destination, document.state.inspectedEntry!!.group) }
    }

    @Test fun reviewedRestoreAndPurgeLeaveTheDatabaseUnchangedUntilConfirmation() {
        val source = document.group("PUBLIC lifecycle trash source")
        val destination = document.group("PUBLIC lifecycle restore destination")
        val original = document.entry(source, current)
        trashEntry()
        document.await { document.state.overview?.entries?.isEmpty() == true }
        openTrash()
        compose.onNodeWithTag("trash-$original").assertExists()
        val trashed = document.savedBytes()

        prepareRestore(original, destination)
        assertArrayEquals(trashed, document.savedBytes())
        cancelReview()
        compose.onNodeWithTag("trash-$original").assertExists()
        assertArrayEquals(trashed, document.savedBytes())
        prepareRestore(original, destination)
        compose.onNodeWithTag("confirm-trash-review").assertIsEnabled().performClick()
        document.waitAbsent("trash-review")
        document.await { !document.state.lifecycle.busy && document.state.lifecycle.rows.none { it.target == original } && document.state.overview?.entries?.singleOrNull()?.group == destination }
        compose.onNodeWithTag("trash-$original").assertDoesNotExist()
        compose.onNodeWithTag("navigation-back").performClick()
        document.waitFor("group-menu")
        document.reopen()
        document.inspect(destination, original)
        assertCurrentFields(original)
        compose.runOnIdle { assertEquals(destination, document.state.inspectedEntry!!.group) }

        trashEntry()
        openTrash()
        val retained = document.savedBytes()
        preparePurge(original)
        assertArrayEquals(retained, document.savedBytes())
        cancelReview()
        compose.onNodeWithTag("trash-$original").assertExists()
        assertArrayEquals(retained, document.savedBytes())
        preparePurge(original)
        compose.onNodeWithTag("confirm-trash-review").assertIsEnabled().performClick()
        document.waitAbsent("trash-review")
        document.await { !document.state.lifecycle.busy && document.state.lifecycle.rows.isEmpty() }
        compose.onNodeWithTag("trash-$original").assertDoesNotExist()
        compose.onNodeWithText(document.text(R.string.trash_empty)).assertExists()
        compose.onNodeWithTag("navigation-back").performClick()
        document.waitFor("group-menu")
        document.reopen()
        compose.runOnIdle { assertTrue(document.state.overview!!.entries.isEmpty()) }
        openTrash()
        compose.onNodeWithTag("trash-$original").assertDoesNotExist()
        compose.onNodeWithText(document.text(R.string.trash_empty)).assertExists()
        compose.runOnIdle { assertTrue(document.state.lifecycle.rows.isEmpty()) }
    }
}
