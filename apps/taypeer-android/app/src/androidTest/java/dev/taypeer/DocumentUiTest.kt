package dev.taypeer

import android.net.Uri
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import dev.taypeer.bridge.EntryTextField
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.UUID

/** Actual Compose actions use supervised Rust workers and encrypted durable files. */
class DocumentUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val password = "PUBLIC UI master password"
    private var database = ""
    private fun text(resource: Int) = compose.activity.getString(resource)
    private fun waitFor(tag: String) = compose.waitUntil(20_000) {
        compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
    }
    private fun createDatabase(): String {
        val name = "PUBLIC UI ${UUID.randomUUID().toString().take(8)}"
        compose.onNodeWithTag("create-database").performClick()
        compose.onNodeWithTag("database-name").performTextReplacement(name)
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("repeat-master-password").performTextReplacement(password)
        compose.onNodeWithTag("authenticate").performClick()
        waitFor("add-entry")
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            state.catalog.any { it.database == state.overview?.database }
        }
        compose.runOnIdle { database = compose.activity.documentState.overview!!.database }
        return name
    }
    private fun savedFile(): File {
        var path = ""
        compose.runOnIdle { path = compose.activity.documentState.catalog.single { it.database == database }.path }
        return File(path)
    }
    private fun createGroup() {
        compose.onNodeWithTag("add-group").performClick()
        waitFor("metadata-name")
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC UI group")
        compose.onNodeWithTag("save-form").performClick()
        waitFor("add-entry")
        compose.onNodeWithTag("add-entry").assertIsEnabled()
    }
    private fun create() { createDatabase(); createGroup() }
    private fun add() {
        compose.onNodeWithTag("add-entry").performClick()
        waitFor("entry-title")
    }
    private fun edit() {
        compose.onNodeWithTag("edit-entry").performScrollTo().performClick()
        compose.onNodeWithTag("entry-tab-0").performClick()
        waitFor("entry-title")
    }
    private fun save() {
        compose.onNodeWithTag("save-form").performClick()
        waitFor("edit-entry")
        compose.onNodeWithTag("save-form").assertDoesNotExist()
    }
    private fun back() {
        compose.onNodeWithText(text(R.string.back)).performScrollTo().performClick()
    }
    private fun unlock() {
        waitFor("create-database")
        compose.onNodeWithTag("catalog-$database").performScrollTo().performClick()
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("authenticate").performClick()
    }
    private fun lock() {
        compose.onNodeWithText(text(R.string.lock)).performClick()
        waitFor("create-database")
    }
    private fun acknowledgedInput() = compose.waitUntil(20_000) {
        compose.activity.documentState.activeForm?.dirty == true && !compose.activity.documentState.navigating
    }
    private fun quiet() { Thread.sleep(800); compose.waitForIdle() }

    @Test fun firstGroupIsRequiredAndTypingDoesNotCommitUntilSave() {
        createDatabase()
        compose.onNodeWithTag("add-entry").assertIsNotEnabled()
        compose.onNodeWithTag("create-group-first").assertExists()
        val empty = savedFile().readBytes()
        compose.onNodeWithTag("add-group").performClick()
        waitFor("metadata-name")
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC explicit group")
        acknowledgedInput(); quiet()
        assertArrayEquals(empty, savedFile().readBytes())
        compose.runOnIdle { assertTrue(compose.activity.documentState.overview!!.groups.isEmpty()) }
        compose.onNodeWithTag("save-form").performClick()
        waitFor("add-entry")
        val grouped = savedFile().readBytes()
        add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC explicit entry")
        acknowledgedInput(); quiet()
        compose.onNodeWithTag("entry-title").assertIsFocused().assertTextContains("PUBLIC explicit entry")
        assertArrayEquals(grouped, savedFile().readBytes())
        save()
        assertFalse(grouped.contentEquals(savedFile().readBytes()))
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.entryHistory.size == 1 }
        lock(); unlock()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC explicit entry")).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun backOffersStayDiscardAndSaveWithoutImplicitWrites() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC original")
        save(); edit()
        val before = savedFile().readBytes()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC rejected edit")
        back(); waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-stay").performClick()
        compose.onNodeWithTag("entry-title").assertTextContains("PUBLIC rejected edit")
        assertArrayEquals(before, savedFile().readBytes())
        compose.runOnIdle { compose.activity.onBackPressedDispatcher.onBackPressed() }
        waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-discard").performClick()
        waitFor("add-entry")
        compose.onNodeWithText("PUBLIC original").assertExists()
        compose.onNodeWithText("PUBLIC rejected edit").assertDoesNotExist()
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithText("PUBLIC original").performClick(); waitFor("edit-entry"); edit()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC accepted edit")
        back(); waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-save").performClick()
        waitFor("add-entry")
        compose.onNodeWithText("PUBLIC accepted edit").assertExists()
        lock(); unlock()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC accepted edit")).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun invalidSaveStaysAndCancelDeletesOnlyTheLocalForm() {
        create(); add()
        val before = savedFile().readBytes()
        compose.onNodeWithTag("entry-username").performTextReplacement("PUBLIC unfinished")
        compose.onNodeWithTag("save-form").performClick()
        waitFor("invalid-form")
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC unfinished")
        assertArrayEquals(before, savedFile().readBytes())
        back(); waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-save").performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.invalidInput && !compose.activity.documentState.navigating }
        compose.onNodeWithTag("unsaved-changes").assertExists()
        compose.onNodeWithTag("navigation-stay").performClick()
        compose.onNodeWithTag("cancel-form").performClick()
        waitFor("add-entry")
        assertArrayEquals(before, savedFile().readBytes())
        compose.runOnIdle {
            assertTrue(compose.activity.documentState.overview!!.entries.isEmpty())
            assertTrue(compose.activity.documentState.forms.isEmpty())
        }
    }

    @Test fun saveCapturesClickInputAndLaterTypingRemainsUnsaved() {
        create(); add()
        compose.runOnIdle {
            val state = compose.activity.documentState
            state.edit(EntryTextField.TITLE, "PUBLIC captured input")
            state.saveNow()
            state.edit(EntryTextField.USERNAME, "PUBLIC later input")
        }
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            state.overview!!.entries.size == 1 && state.activeForm?.dirty == true && state.saveLabel == SaveLabel.DIRTY
        }
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC later input")
        val captured = savedFile().readBytes()
        quiet()
        assertArrayEquals(captured, savedFile().readBytes())
        compose.onNodeWithTag("cancel-form").performClick()
        waitFor("edit-entry")
        compose.runOnIdle { assertNull(compose.activity.documentState.inspectedEntry!!.username) }
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.entryHistory.size == 1 }
    }

    @Test fun backgroundRetainsAnEncryptedDraftWithoutSavingTheDatabase() {
        create(); add()
        compose.onNodeWithTag("entry-username").performTextReplacement("PUBLIC local unfinished")
        acknowledgedInput()
        var draft = ""
        compose.runOnIdle { draft = compose.activity.documentState.activeForm!!.draft }
        val before = savedFile().readBytes()
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        waitFor("create-database")
        compose.onNodeWithTag("entry-username").assertDoesNotExist()
        compose.onNodeWithTag("revealed-secret").assertDoesNotExist()
        quiet()
        compose.onNodeWithTag("entry-title").assertDoesNotExist()
        unlock(); waitFor("entry-username")
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC local unfinished")
        compose.runOnIdle { assertEquals(draft, compose.activity.documentState.activeForm!!.draft) }
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("cancel-form").performClick(); waitFor("add-entry")
    }

    @Test fun tabsAttributesAndGeneratorKeepOneUncommittedForm() {
        create(); add()
        val before = savedFile().readBytes()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC all tabs")
        compose.onNodeWithTag("entry-tags").performScrollTo().performTextReplacement("PUBLIC tag one\nPUBLIC tag two")
        compose.onNodeWithTag("entry-tab-1").performClick()
        compose.onNodeWithText(text(R.string.add_attribute)).performScrollTo().performClick()
        compose.onNodeWithTag("new-attribute-name").performTextReplacement("PUBLIC attribute")
        compose.onNodeWithTag("new-attribute-value").performTextReplacement("PUBLIC attribute value")
        compose.onNodeWithText(text(R.string.apply)).performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.entry?.source?.attributes?.size == 1 }
        var attribute = ""
        compose.runOnIdle { attribute = compose.activity.documentState.entry!!.source.attributes.single().id!! }
        compose.onNodeWithTag("attribute-value-$attribute").performScrollTo().performTextReplacement("PUBLIC updated attribute")
        compose.onNodeWithTag("attribute-protected-$attribute").performScrollTo().performClick()
        compose.onNodeWithTag("entry-tab-2").performScrollTo().performClick()
        compose.onNodeWithTag("icon-picker").performScrollTo().performClick()
        waitFor("icon-key-key-round")
        compose.onNodeWithTag("icon-key-key-round").performScrollTo().performClick()
        acknowledgedInput(); quiet()
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("open-generator").performClick()
        compose.onNodeWithText(text(R.string.library)).performClick()
        assertArrayEquals(before, savedFile().readBytes())
        save()
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.entryHistory.size == 1 }
        lock(); unlock()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC all tabs")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("PUBLIC all tabs").performClick(); waitFor("edit-entry")
        compose.onNodeWithTag("entry-tab-1").performClick()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC updated attribute")).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun groupAndDatabaseMetadataRequireTheirOwnSaveButton() {
        create()
        var group = ""
        compose.runOnIdle { group = compose.activity.documentState.selectedGroup!! }
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithTag("edit-group-$group").performClick(); waitFor("metadata-name")
        val before = savedFile().readBytes()
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC renamed group")
        compose.onNodeWithTag("metadata-description").performTextReplacement("PUBLIC group description")
        acknowledgedInput(); quiet()
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("save-form").performClick(); waitFor("add-entry")
        compose.onNode(hasText("PUBLIC renamed group") and !hasClickAction()).assertExists()
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithText(text(R.string.database_info)).performClick(); waitFor("metadata-name")
        val grouped = savedFile().readBytes()
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC renamed database")
        acknowledgedInput(); quiet()
        assertArrayEquals(grouped, savedFile().readBytes())
        compose.onNodeWithTag("save-form").performClick(); waitFor("add-entry")
        compose.onNodeWithText("PUBLIC renamed database").assertExists()
    }

    @Test fun safPinAndAcceptedAttachmentStayLocalUntilExplicitSave() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC selected-file entry")
        acknowledgedInput()
        val before = savedFile().readBytes()
        val originalDatabase = database
        val name = "PUBLIC-${UUID.randomUUID()}.bin"
        val folder = File(compose.activity.cacheDir, "PUBLIC-selected-fixtures").apply { mkdirs() }
        val source = File(folder, name).apply { writeBytes(ByteArray(131_073) { (it % 241).toByte() }) }
        try {
            val uri = Uri.parse("content://dev.taypeer.publicselected/$name")
            // This is the real picker callback with a synthetic content provider;
            // no session or plaintext capability is carried across background.
            compose.runOnIdle { compose.activity.documentState.prepareAttachment(null) { operation -> compose.activity.documentState.selectedAttachment(operation, uri) } }
            waitFor("accept-selected-file")
            assertArrayEquals(before, savedFile().readBytes())
            lock()
            compose.onNodeWithTag("accept-selected-file").assertIsNotEnabled()
            createDatabase()
            compose.onNodeWithTag("accept-selected-file").assertIsNotEnabled()
            lock(); database = originalDatabase
            unlock(); waitFor("entry-title")
            compose.onNodeWithTag("entry-title").assertTextContains("PUBLIC selected-file entry")
            compose.onNodeWithTag("accept-selected-file").performScrollTo().assertIsEnabled().performClick()
            compose.onNodeWithTag("entry-tab-1").performClick()
            compose.waitUntil(20_000) { compose.onAllNodes(hasText(name)).fetchSemanticsNodes().isNotEmpty() }
            quiet()
            assertArrayEquals(before, savedFile().readBytes())
            save()
            compose.waitUntil(20_000) { compose.onAllNodes(hasText(name)).fetchSemanticsNodes().isNotEmpty() }
            lock(); unlock()
            compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC selected-file entry")).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("PUBLIC selected-file entry").performClick(); waitFor("edit-entry")
            compose.onNodeWithTag("entry-tab-1").performClick()
            compose.waitUntil(20_000) { compose.onAllNodes(hasText(name)).fetchSemanticsNodes().isNotEmpty() }
        } finally { source.delete() }
    }
}
