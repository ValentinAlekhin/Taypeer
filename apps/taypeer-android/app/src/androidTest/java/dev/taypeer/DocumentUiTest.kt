package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import org.junit.Rule
import org.junit.Test
import java.util.UUID
import android.net.Uri
import java.io.File

/** Actual Compose input drives a supervised isolated Rust process and durable encrypted files. */
class DocumentUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val password = "PUBLIC UI master password"
    private var database = ""
    private fun text(resource: Int) = compose.activity.getString(resource)
    private fun waitFor(tag: String) = compose.waitUntil(20_000) {
        compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
    }
    private fun create(): String {
        val name = "PUBLIC UI ${UUID.randomUUID().toString().take(8)}"
        compose.onNodeWithTag("create-database").performClick()
        compose.onNodeWithTag("database-name").performTextReplacement(name)
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("repeat-master-password").performTextReplacement(password)
        compose.onNodeWithTag("authenticate").performClick()
        waitFor("add-entry")
        compose.runOnIdle { database = compose.activity.documentState.overview!!.database }
        return name
    }
    private fun add() {
        compose.onNodeWithTag("add-entry").performClick()
        waitFor("entry-title")
    }
    private fun back() {
        compose.onNodeWithText(text(R.string.back)).performScrollTo().performClick()
        waitFor("add-entry")
    }
    private fun lockAndUnlock() {
        compose.onNodeWithText(text(R.string.lock)).performClick()
        waitFor("create-database")
        compose.onNodeWithTag("catalog-$database").performScrollTo().performClick()
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("authenticate").performClick()
    }
    private fun saved() = compose.waitUntil(20_000) {
        compose.onAllNodes(hasTestTag("save-status") and hasText(text(R.string.save_saved))).fetchSemanticsNodes().isNotEmpty()
    }
    @Test fun quietAutosaveKeepsFocusAndReopensActualDurableContent() {
        create(); add()
        val entered = "PUBLIC actual autosaved entry"
        compose.onNodeWithTag("entry-title").performTextReplacement(entered)
        saved()
        compose.onNodeWithTag("entry-title").assertIsFocused().assertTextContains(entered)
        compose.onNodeWithTag("entry-username").performTextReplacement("PUBLIC later username")
        // Immediate navigation flushes without waiting for the quiet timer.
        back()
        compose.onNodeWithText(entered).assertExists()
        compose.onNodeWithText(entered).performClick()
        waitFor("edit-entry")
        compose.onNodeWithTag("edit-entry").performScrollTo().performClick()
        waitFor("entry-title")
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC later username")
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC later username")
        lockAndUnlock()
        compose.waitUntil(20_000) {
            compose.onAllNodes(hasTestTag("entry-username") or hasText(entered)).fetchSemanticsNodes().isNotEmpty()
        }
        if (compose.onAllNodes(hasTestTag("entry-username")).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText(entered).performClick()
            waitFor("edit-entry")
            compose.onNodeWithTag("edit-entry").performScrollTo().performClick()
        }
        waitFor("entry-username")
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC later username")
        compose.onNodeWithText(text(R.string.lock)).performClick()
        compose.onNodeWithTag("entry-title").assertDoesNotExist()
    }
    @Test fun independentIncompleteFormsAutomaticallyResumeExactActiveIdentity() {
        create(); add()
        compose.onNodeWithTag("entry-username").performTextReplacement("PUBLIC first unfinished")
        back(); add()
        compose.onNodeWithTag("entry-username").performTextReplacement("PUBLIC second unfinished")
        compose.waitUntil(20_000) {
            compose.onAllNodes(hasTestTag("save-status") and hasText(text(R.string.save_local))).fetchSemanticsNodes().isNotEmpty()
        }
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        waitFor("create-database")
        compose.onNodeWithTag("entry-username").assertDoesNotExist()
        // The same user selects the public working-copy identity then authenticates.
        compose.onNodeWithTag("catalog-$database").performScrollTo().performClick()
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("authenticate").performClick()
        waitFor("entry-username")
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC second unfinished")
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithText(text(R.string.local_forms)).performClick()
        compose.waitUntil(10_000) { compose.onAllNodes(hasText(text(R.string.new_entry), substring = true)).fetchSemanticsNodes().size == 2 }
        compose.onNodeWithText(text(R.string.close)).performClick()
        compose.onNodeWithText(text(R.string.lock)).performClick()
    }
    @Test fun backgroundDuringTypingCannotReturnLateInputOrRevealedValues() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC background race")
        compose.onNodeWithTag("entry-password").performScrollTo().performTextReplacement("PUBLIC protected UI sentinel")
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        waitFor("create-database")
        compose.onNodeWithTag("entry-password").assertDoesNotExist()
        compose.onNodeWithTag("revealed-secret").assertDoesNotExist()
        Thread.sleep(1200) // Outlast the debounce and any response already queued at background.
        compose.onNodeWithTag("entry-title").assertDoesNotExist()
        compose.onNodeWithTag("entry-password").assertDoesNotExist()
    }
    @Test fun tagsAppearancePropertiesAndOriginalHistorySurviveWorkerRestart() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC five-tab entry")
        compose.onNodeWithTag("entry-tags").performScrollTo().performTextReplacement("PUBLIC tag one\nPUBLIC tag two")
        saved()
        compose.onNodeWithTag("entry-tab-2").performScrollTo().performClick()
        waitFor("icon-picker")
        compose.onNodeWithTag("icon-picker").performScrollTo().performClick()
        waitFor("icon-key-key-round")
        compose.onNodeWithTag("icon-key-key-round").performScrollTo().performClick()
        saved()
        lockAndUnlock()
        compose.waitUntil(20_000) {
            compose.onAllNodes(hasTestTag("icon-picker") or hasText("PUBLIC five-tab entry")).fetchSemanticsNodes().isNotEmpty()
        }
        if (compose.onAllNodes(hasTestTag("icon-picker")).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText("PUBLIC five-tab entry").performClick()
            waitFor("edit-entry")
            compose.onNodeWithTag("edit-entry").performScrollTo().performClick()
        }
        waitFor("icon-picker")
        compose.waitUntil(20_000) { compose.onAllNodes(hasTestTag("icon-picker") and hasText("key-round", substring = true)).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("icon-picker").assertTextContains("key-round", substring = true)
        compose.onNodeWithTag("entry-tab-3").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC tag one\nPUBLIC tag two")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText(text(R.string.created_at)).assertExists()
        compose.onNodeWithText(text(R.string.modified_at)).assertExists()
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC five-tab entry")).fetchSemanticsNodes().isNotEmpty() }
        // History contains the original text snapshot and the separate icon edit.
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC five-tab entry")).fetchSemanticsNodes().size >= 2 }
        compose.onNodeWithText(text(R.string.lock)).performClick()
        compose.onNodeWithTag("icon-picker").assertDoesNotExist()
    }
    @Test fun attributeValueAndProtectionAutosaveWithoutApplyAndReopenDurably() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC attribute entry")
        saved()
        compose.onNodeWithTag("entry-tab-1").performClick()
        compose.onNodeWithText(text(R.string.add_attribute)).performScrollTo().performClick()
        compose.onNodeWithTag("new-attribute-name").performTextReplacement("PUBLIC attribute")
        compose.onNodeWithTag("new-attribute-value").performTextReplacement("PUBLIC initial attribute")
        compose.onNodeWithText(text(R.string.apply)).performClick()
        saved()
        var attribute = ""
        compose.runOnIdle { attribute = compose.activity.documentState.entry!!.source.attributes.single().id!! }
        compose.onNodeWithTag("attribute-value-$attribute").performScrollTo().performTextReplacement("PUBLIC quiet attribute")
        saved()
        compose.onNodeWithTag("attribute-value-$attribute").assertIsFocused().assertTextContains("PUBLIC quiet attribute")
        compose.onNodeWithTag("attribute-protected-$attribute").performScrollTo().performClick()
        saved()
        lockAndUnlock()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC attribute entry") or hasTestTag("attribute-value-$attribute")).fetchSemanticsNodes().isNotEmpty() }
        if (compose.onAllNodes(hasTestTag("attribute-value-$attribute")).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText("PUBLIC attribute entry").performClick()
            waitFor("edit-entry")
        }
        compose.onNodeWithTag("entry-tab-1").performClick()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC quiet attribute")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("PUBLIC quiet attribute").assertExists()
        compose.onNodeWithText(text(R.string.lock)).performClick()
    }

    @Test fun scriptedSafResultRequiresFreshUnlockOfItsExactDraftBeforeAutosave() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC selected-file entry")
        saved()
        val originalDatabase = database
        val name = "PUBLIC-${UUID.randomUUID()}.bin"
        val folder = File(compose.activity.cacheDir, "PUBLIC-selected-fixtures").apply { mkdirs() }
        File(folder, name).writeBytes(ByteArray(131_073) { (it % 241).toByte() })
        val uri = Uri.parse("content://dev.taypeer.publicselected/$name")
        // Invoke the same picker completion with a private synthetic provider. The actual
        // lifecycle transition below still revokes the old isolated process and capabilities.
        compose.runOnIdle { compose.activity.documentState.prepareAttachment(null) { operation -> compose.activity.documentState.selectedAttachment(operation, uri) } }
        waitFor("accept-selected-file")
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        waitFor("create-database")
        compose.onNodeWithTag("accept-selected-file").assertIsNotEnabled()
        create() // A different database cannot inherit a pending file capability.
        compose.onNodeWithTag("accept-selected-file").assertIsNotEnabled()
        compose.onNodeWithText(text(R.string.lock)).performClick()
        database = originalDatabase
        waitFor("create-database")
        compose.onNodeWithTag("catalog-$database").performScrollTo().performClick()
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("authenticate").performClick()
        waitFor("entry-title")
        compose.onNodeWithTag("entry-title").assertTextContains("PUBLIC selected-file entry")
        compose.onNodeWithTag("accept-selected-file").performScrollTo().assertIsEnabled().performClick()
        saved()
        compose.onNodeWithTag("entry-tab-1").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.onAllNodes(hasText(name)).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText(text(R.string.rename)).performScrollTo().performClick()
        compose.onNodeWithTag("attachment-name").performTextReplacement("PUBLIC renamed attachment.bin")
        compose.onNodeWithText(text(R.string.apply)).performClick()
        saved()
        lockAndUnlock()
        compose.waitUntil(20_000) {
            compose.onAllNodes(hasText("PUBLIC renamed attachment.bin") or hasText("PUBLIC selected-file entry")).fetchSemanticsNodes().isNotEmpty()
        }
        if (compose.onAllNodes(hasText("PUBLIC renamed attachment.bin")).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText("PUBLIC selected-file entry").performClick()
        }
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC renamed attachment.bin")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("131073 ${text(R.string.bytes)}").assertExists()
        compose.onNodeWithText(text(R.string.lock)).performClick()
        File(folder, name).delete()
    }
}
