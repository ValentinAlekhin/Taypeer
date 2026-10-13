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
    private var groupBoundsBeforeClick = "not clicked"
    private fun text(resource: Int) = compose.activity.getString(resource)
    private fun waitFor(tag: String) {
        try { compose.waitUntil(20_000) { compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty() } }
        catch (error: ComposeTimeoutException) {
            var state = ""
            compose.runOnIdle { compose.activity.documentState.let { current ->
                state = "overview=${current.overview != null}, metadata=${current.metadata != null}, entry=${current.entry != null}, inspected=${current.inspectedEntry != null}, opening=${current.opening}, navigating=${current.navigating}, failure=${current.error}, invalid=${current.invalidInput}, decision=${current.navigationDecision}, groups=${current.overview?.groups?.size}, entries=${current.overview?.entries?.size}, forms=${current.forms.size}, active=${current.activeForm?.kind}, writable=${current.overview?.writable}, selected=${current.selectedGroup != null}, history=${current.history != null}, exchange=${current.exchange != null}, policy=${current.policy != null}"
            } }
            val groupBounds = compose.onAllNodes(hasTestTag("add-group")).fetchSemanticsNodes().map { it.boundsInRoot }
            val routes = listOf("route-entries", "route-welcome", "route-settings", "route-metadata", "route-entry", "route-inspected", "route-authentication", "database-drawer-open", "database-drawer-closed", "drawer-add-group", "theme-system", "master-password", "metadata-name", "entry-title", "unsaved-changes").associateWith { safeTag -> compose.onAllNodes(hasTestTag(safeTag)).fetchSemanticsNodes().size }
            throw AssertionError("Missing $tag: $state; group bounds before=$groupBoundsBeforeClick after=$groupBounds; safe routes=$routes", error)
        }
    }
    private fun createDatabase(): String {
        val name = "PUBLIC UI ${UUID.randomUUID().toString().take(8)}"
        if (compose.onAllNodes(hasTestTag("choose-another-database")).fetchSemanticsNodes().isNotEmpty()) compose.onNodeWithTag("choose-another-database").performClick()
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
        startGroup()
        waitFor("metadata-name")
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC UI group")
        compose.onNodeWithTag("save-form").performClick()
        waitFor("add-entry")
        compose.onNodeWithTag("add-entry").assertIsEnabled()
    }
    private fun startGroup() {
        groupBoundsBeforeClick = compose.onNodeWithTag("add-group").fetchSemanticsNode().boundsInRoot.toString()
        compose.onNodeWithTag("add-group").assertIsEnabled().performClick()
    }
    private fun create() { createDatabase(); createGroup() }
    private fun add() {
        compose.onNodeWithTag("add-entry").performClick()
        waitFor("entry-title")
    }
    private fun edit() {
        compose.onNodeWithTag("edit-entry").performClick()
        compose.onNodeWithTag("entry-tab-0").performScrollTo().performClick()
        waitFor("entry-title")
    }
    private fun save() {
        compose.onNodeWithTag("save-form").performClick()
        waitFor("edit-entry")
        compose.onNodeWithTag("save-form").assertDoesNotExist()
    }
    private fun back() {
        compose.onNodeWithTag("navigation-back").performClick()
    }
    private fun unlock() {
        compose.waitUntil(20_000) { compose.onAllNodes(hasTestTag("master-password")).fetchSemanticsNodes().isNotEmpty() || compose.onAllNodes(hasTestTag("create-database")).fetchSemanticsNodes().isNotEmpty() }
        if (compose.activity.documentState.lockedCopy?.database != database && compose.onAllNodes(hasTestTag("choose-another-database")).fetchSemanticsNodes().isNotEmpty())
            compose.onNodeWithTag("choose-another-database").performClick()
        if (compose.onAllNodes(hasTestTag("master-password")).fetchSemanticsNodes().isEmpty()) compose.onNodeWithTag("catalog-$database").performScrollTo().performClick()
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.tapWorkingCopyUnlock(database)
    }
    private fun lock() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("lock-database").performClick()
        waitFor("database-locked")
    }
    private fun acknowledgedInput() = compose.waitUntil(20_000) {
        compose.activity.documentState.activeForm?.dirty == true && !compose.activity.documentState.navigating
    }
    private fun quiet() { Thread.sleep(800); compose.waitForIdle() }

    @Test fun openingAndUnlockingKeepTheDrawerClosedAndGroupCreationTouchable() {
        createDatabase()
        compose.onNodeWithTag("database-drawer-closed").assertExists()
        compose.onNodeWithTag("database-drawer-open").assertDoesNotExist()
        createGroup()
        lock(); unlock()
        waitFor("add-entry")
        compose.onNodeWithTag("database-drawer-closed").assertExists()
        compose.onNodeWithTag("database-drawer-open").assertDoesNotExist()
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithTag("database-drawer-open").assertExists()
        compose.onNodeWithTag("drawer-lock-database").performScrollTo().performClick()
        waitFor("database-locked")
    }

    @Test fun firstGroupIsRequiredAndTypingDoesNotCommitUntilSave() {
        createDatabase()
        compose.onNodeWithTag("add-entry").assertIsNotEnabled()
        compose.onNodeWithTag("create-group-first").assertExists()
        val empty = savedFile().readBytes()
        startGroup()
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
        waitFor("database-locked")
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
        waitFor("icon-source-lucide")
        compose.onNodeWithTag("icon-source-lucide").performClick()
        waitFor("icon-key-key-round")
        compose.onNodeWithTag("icon-key-key-round").performScrollTo().performClick()
        compose.onNodeWithTag("choose-lucide").performClick()
        acknowledgedInput(); quiet()
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-generator").performClick()
        compose.onNodeWithTag("navigation-back").performClick()
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
        compose.onNodeWithTag("edit-group-$group").performClick()
        compose.onNodeWithTag("edit-selected-group").performClick(); waitFor("metadata-name")
        val before = savedFile().readBytes()
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC renamed group")
        compose.onNodeWithTag("metadata-description").performTextReplacement("PUBLIC group description")
        acknowledgedInput(); quiet()
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("save-form").performClick(); waitFor("add-entry")
        compose.onNode(hasText("PUBLIC renamed group") and !hasClickAction()).assertExists()
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithTag("drawer-settings").performClick()
        waitFor("settings-scope")
        compose.onNodeWithTag("settings-scope").performClick()
        compose.onNodeWithTag("settings-scope-database").performClick()
        compose.onNodeWithTag("database-information").performScrollTo().performClick(); waitFor("metadata-name")
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
            compose.onNodeWithTag("accept-selected-file").assertDoesNotExist()
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

    @Test fun searchUsesTheWholeDatabaseAndClearRestoresTheSelectedGroup() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC first group entry")
        save(); back(); waitFor("add-entry")
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithTag("drawer-add-group").performClick(); waitFor("metadata-name")
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC second group")
        compose.onNodeWithTag("save-form").performClick(); waitFor("add-entry")
        add(); compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC second group entry")
        save(); back(); waitFor("entry-search")
        compose.onNodeWithText("PUBLIC first group entry").assertDoesNotExist()
        compose.onNodeWithTag("entry-search").performTextReplacement("PUBLIC first group")
        compose.waitUntil(20_000) { compose.onAllNodes(hasText("PUBLIC first group entry")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("clear-search").performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.query.isEmpty() }
        compose.onNodeWithText("PUBLIC first group entry").assertDoesNotExist()
        compose.onNodeWithText("PUBLIC second group entry").assertExists()
    }

    @Test fun copyOfHiddenPasswordDoesNotRevealItInTheDocumentUi() {
        create(); add()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC hidden copy entry")
        compose.onNodeWithTag("entry-password").performTextReplacement("PUBLIC hidden copy value")
        save()
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            !state.navigating && state.activeForm == null && state.inspectedEntry?.hasPassword == true
        }
        // A physical copy tap must land inside the inspector viewport after the IME closes.
        compose.runOnIdle {
            compose.activity.getSystemService(android.view.inputmethod.InputMethodManager::class.java)
                .hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0)
        }
        compose.waitUntil(10_000) {
            compose.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) != true
        }
        compose.onNodeWithText("PUBLIC hidden copy value").assertDoesNotExist()
        val copyButton = compose.onNodeWithTag("copy-password").performScrollTo().assertIsDisplayed()
        val copyBounds = copyButton.fetchSemanticsNode().boundsInRoot
        copyButton.performTouchInput { click() }
        val clipboard = compose.activity.getSystemService(android.content.ClipboardManager::class.java)
        try {
            compose.waitUntil(20_000) {
                clipboard.primaryClip?.getItemAt(0)?.text?.toString() == "PUBLIC hidden copy value"
            }
        } catch (timeout: ComposeTimeoutException) {
            var flags = ""
            compose.runOnIdle {
                val state = compose.activity.documentState
                val policy = (compose.activity.application as TaypeerApplication).deviceSettings
                flags = "focus=${compose.activity.hasWindowFocus()}, lifecycle=${compose.activity.lifecycle.currentState}, " +
                    "policyLoaded=${policy.settings != null}, policyBusy=${policy.busy}, policyFailed=${policy.failed}, " +
                    "clipboardTimed=${policy.settings?.clipboardSeconds != null}, overview=${state.overview != null}, " +
                    "hasPassword=${state.inspectedEntry?.hasPassword}, revealed=${state.revealed.isNotEmpty()}, " +
                    "navigating=${state.navigating}, failed=${state.error}, invalid=${state.invalidInput}, " +
                    "imeVisible=${compose.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime())}, " +
                    "copyBounds=$copyBounds"
            }
            throw AssertionError("Hidden-copy acknowledgement missing; $flags", timeout)
        }
        compose.runOnIdle {
            assertTrue(compose.activity.documentState.revealed.isEmpty())
            assertEquals("PUBLIC hidden copy value", clipboard.primaryClip?.getItemAt(0)?.text?.toString())
        }
        compose.onNodeWithText("PUBLIC hidden copy value").assertDoesNotExist()
        compose.onNodeWithTag("reveal-password").performScrollTo().assertIsDisplayed().performTouchInput { click() }
        compose.waitUntil(20_000) { compose.activity.documentState.revealed == "PUBLIC hidden copy value" }
        compose.onNodeWithText("PUBLIC hidden copy value").assertExists()
        compose.onNodeWithTag("entry-tab-1").performClick()
        compose.runOnIdle { assertTrue(compose.activity.documentState.revealed.isEmpty()) }
    }
}
