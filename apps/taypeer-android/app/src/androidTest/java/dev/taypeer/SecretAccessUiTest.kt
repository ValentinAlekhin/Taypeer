package dev.taypeer

import android.content.ClipboardManager
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.Lifecycle
import dev.taypeer.bridge.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/** Real supervised documents and actual drawing/copy callbacks; no fabricated grants or timestamps. */
class SecretAccessUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val application get() = compose.activity.application as TaypeerApplication
    private val state get() = document.state
    private val fields = PublicEntryFields(title = "PUBLIC access UI entry", password = "PUBLIC access UI password")
    private val observer = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val confirmations = AtomicInteger()
    private var database = ""
    private var copiedProperty = ""
    private var copiedGenerator = ""
    private val clipboard get() = compose.activity.getSystemService(ClipboardManager::class.java)

    @Before fun createPublicDocument() {
        document = PublicDocumentUiFixture(compose)
        document.create()
        compose.runOnIdle { database = state.overview!!.database }
        observer.launch { application.secretAccess.confirmations.collect { if (it.target.database == database) confirmations.incrementAndGet() } }
        compose.waitForIdle()
    }

    @After fun releaseOnlyPublicFixtures() {
        observer.cancel()
        if (!::document.isInitialized) return
        compose.runOnIdle {
            val value = clipboard.primaryClip?.takeIf { it.itemCount == 1 }?.getItemAt(0)?.text?.toString()
            if (value in setOf(fields.password, fields.username, "PUBLIC protected access value", "PUBLIC current access UI password", copiedProperty, copiedGenerator)) clipboard.clearPrimaryClip()
        }
        document.restorePreferences()
    }

    private fun savedEntry(): String = document.entry(document.group("PUBLIC access UI group"), fields)
    private fun awaitEvents(count: Int) = document.await { confirmations.get() == count }
    private fun tab(index: Int) { compose.onNodeWithTag("entry-tab-$index").performScrollTo().performClick() }

    @Test fun existingEditorCurrentProtectedHistoryAndComparisonActionsRecordTheSameEntryWithoutChangingTheDocument() {
        val entry = savedEntry()
        compose.runOnIdle { state.beginEntry(entry) }
        document.await { state.entry != null && !state.navigating }
        compose.runOnIdle { state.editAttribute(null, "PUBLIC protected access attribute", "PUBLIC protected access value", true) }
        document.await { state.entry?.source?.attributes?.any { it.protected && it.id != null } == true }
        var protected = ""
        compose.runOnIdle { protected = state.entry!!.source.attributes.single { it.protected }.id!!; state.saveNow() }
        document.await { state.activeForm == null && state.inspectedEntry?.id == entry }
        tab(4)
        document.await { state.entryHistory.size == 2 }
        var revision = ""
        compose.runOnIdle { revision = state.entryHistory.last().id }
        document.update(entry, fields.copy(title = "PUBLIC current access UI title", password = "PUBLIC current access UI password"))
        tab(0)
        val before = document.savedBytes()
        var originalProperties: EntryProperties? = null
        compose.runOnIdle { originalProperties = state.properties; assertNull(state.properties!!.accessedAt) }
        compose.onNodeWithTag("edit-entry").performClick()
        document.waitFor("entry-password")
        compose.runOnIdle { assertTrue(state.entry!!.password.isEmpty()); assertEquals(entry, state.entry!!.source.entry) }
        compose.onNodeWithTag("reveal-entry-input-password").performScrollTo().performClick()
        awaitEvents(1)
        compose.onNodeWithTag("entry-password").assertRenderedText("PUBLIC current access UI password")
        tab(1)
        compose.onNodeWithTag("reveal-editor-attribute-$protected").performScrollTo().performClick()
        document.waitFor("revealed-secret")
        compose.onNodeWithTag("revealed-secret").performScrollTo()
        awaitEvents(2)
        compose.onNodeWithTag("revealed-secret").assertTextEquals("PUBLIC protected access value")
        document.await { application.deviceSettings.settings != null && compose.activity.hasWindowFocus() }
        compose.onNodeWithTag("copy-revealed-secret").performScrollTo().performClick()
        awaitEvents(3)
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == "PUBLIC protected access value" }
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("cancel-form").performClick()
        document.await { state.entry == null && state.activeForm == null && state.inspectedEntry?.id == entry && !state.navigating }
        tab(0)
        compose.onNodeWithTag("copy-username").performScrollTo().performClick()
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == fields.username }
        compose.onRoot().captureToImage()
        assertEquals(3, confirmations.get())

        tab(1)
        document.waitFor("reveal-attribute-$protected")
        compose.onNodeWithTag("reveal-attribute-$protected").performScrollTo().performClick()
        awaitEvents(4)
        compose.onNodeWithText("PUBLIC protected access value").assertIsDisplayed()
        tab(0)
        compose.onNodeWithTag("reveal-password").performScrollTo().performClick()
        awaitEvents(5)
        compose.onNodeWithText("PUBLIC current access UI password").assertIsDisplayed()
        compose.onNodeWithTag("reveal-password").performClick()
        document.await { state.revealed.isEmpty() }
        assertEquals(5, confirmations.get())
        compose.onNodeWithTag("copy-password").performClick()
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == "PUBLIC current access UI password" }
        awaitEvents(6)

        compose.runOnIdle { state.showHistory(FormKind.ENTRY, entry) }
        document.await { state.history?.rows?.size == 3 && !state.navigating }
        compose.onNodeWithTag("history-version-$revision").performScrollTo().performClick()
        document.waitFor("saved-version-tab-0")
        compose.onNodeWithTag("reveal-password").performScrollTo().performClick()
        awaitEvents(7)
        compose.onNodeWithText(fields.password).assertIsDisplayed()
        compose.onNodeWithTag("compare-version").performScrollTo().performClick()
        document.await { state.comparison.saved != null && state.comparison.current != null && !state.comparison.busy }
        compose.onNodeWithTag("reveal-comparison-saved").performScrollTo().performClick()
        awaitEvents(8)
        compose.onNodeWithTag("comparison-saved-password").assertTextContains(fields.password)
        compose.onNodeWithTag("reveal-comparison-current").performScrollTo().performClick()
        awaitEvents(9)
        compose.onNodeWithTag("comparison-current-password").assertTextContains("PUBLIC current access UI password")
        compose.runOnIdle {
            assertNotNull(state.historicalProperties!!.accessedAt)
            assertEquals(originalProperties!!.createdAt, state.historicalProperties!!.createdAt)
            assertNull(state.activeForm)
            state.comparison.close()
        }
        document.waitFor("saved-version-tab-0")
        tab(3)
        document.waitFor("copy-property-access")
        var time = 0L
        compose.runOnIdle { time = state.historicalProperties!!.accessedAt!!; copiedProperty = displayTime(time) }
        compose.onNodeWithTag("copy-property-access").performScrollTo().performClick()
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == displayTime(time) }
        assertEquals(9, confirmations.get())
        assertArrayEquals(before, document.savedBytes())
        compose.runOnIdle { assertEquals(3, state.history!!.rows.size); state.dismissHistory() }
        document.reopen()
        var group = ""
        compose.runOnIdle { group = state.overview!!.groups.single().id }
        document.inspect(group, entry)
        compose.runOnIdle { assertEquals(time, state.properties!!.accessedAt); assertEquals(originalProperties!!.modifiedAt, state.properties!!.modifiedAt) }
        assertArrayEquals(before, document.savedBytes())
    }

    @Test fun offscreenReplyAndHiddenBeforeDrawDoNotRecordAccess() {
        savedEntry()
        val before = document.savedBytes()
        compose.activityRule.scenario.onActivity { activity -> activity.setContent {
            TaypeerTheme {
                Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                    Spacer(Modifier.height(2000.dp))
                    Column(Modifier.testTag("PUBLIC-offscreen-secret")) {
                        state.inspectedEntry?.let { row -> EntrySummary(row, emptyList(), null, state.revealed.ifEmpty { null },
                            { if (state.revealed.isEmpty()) state.reveal() else state.hideSecrets() }, {}, {}, display = state.secretDisplay) }
                    }
                }
            }
        } }
        compose.runOnIdle { state.reveal() }
        document.await { state.revealed == fields.password }
        compose.onRoot().captureToImage()
        compose.runOnIdle {
            assertNull(state.properties!!.accessedAt)
            assertEquals(Lifecycle.State.RESUMED, compose.activity.lifecycle.currentState)
            compose.activity.onWindowFocusChanged(false)
            assertEquals(Lifecycle.State.RESUMED, compose.activity.lifecycle.currentState)
            assertTrue(state.revealed.isEmpty())
            assertNull(state.secretDisplay)
            compose.activity.onWindowFocusChanged(true)
        }
        assertEquals(0, confirmations.get())
        compose.onNodeWithTag("PUBLIC-offscreen-secret").performScrollTo()
        compose.onRoot().captureToImage()
        assertEquals(0, confirmations.get())
        compose.onNodeWithTag("reveal-password").performScrollTo().performClick()
        awaitEvents(1)
        compose.onNodeWithText(fields.password).assertIsDisplayed()
        assertArrayEquals(before, document.savedBytes())
    }

    @Test fun failedCopyUnsavedEditorsAndGeneratorDoNotRecordAccess() {
        savedEntry()
        val before = document.savedBytes()
        val copies = AtomicInteger()
        compose.runOnIdle { state.copySecret({ copies.incrementAndGet(); false }) }
        document.await { copies.get() == 1 }
        compose.onRoot().captureToImage()
        compose.runOnIdle { assertNull(state.properties!!.accessedAt) }
        assertEquals(0, confirmations.get())
        compose.onNodeWithTag("edit-entry").performClick()
        document.waitFor("entry-password")
        compose.onNodeWithTag("entry-password").performTextReplacement("PUBLIC unsaved access password")
        compose.onNodeWithTag("reveal-entry-input-password").performScrollTo().performClick()
        compose.onNodeWithTag("entry-password").assertRenderedText("PUBLIC unsaved access password")
        compose.runOnIdle { assertNull(state.secretDisplay) }
        assertEquals(0, confirmations.get())
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("cancel-form").performClick()
        document.await { state.entry == null && state.activeForm == null }
        compose.runOnIdle { assertNull(state.properties!!.accessedAt) }
        compose.runOnIdle { state.chooseGroup(state.selectedGroup) }
        document.await { state.inspectedEntry == null && !state.navigating }
        compose.runOnIdle { state.beginEntry() }
        document.waitFor("entry-password")
        compose.onNodeWithTag("entry-password").performTextReplacement("PUBLIC new unsaved access password")
        compose.onNodeWithTag("reveal-entry-input-password").performScrollTo().performClick()
        compose.onNodeWithTag("entry-password").assertRenderedText("PUBLIC new unsaved access password")
        compose.runOnIdle { assertNull(state.entry!!.source.entry); assertNull(state.secretDisplay) }
        assertEquals(0, confirmations.get())
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("cancel-form").performClick()
        document.await { state.entry == null && state.activeForm == null }
        document.menu("open-generator")
        document.waitFor("generated-password")
        document.await { !compose.activity.generatorState.running && compose.activity.generatorState.generated.isNotEmpty() }
        compose.runOnIdle { copiedGenerator = compose.activity.generatorState.generated }
        compose.onNodeWithTag("generator-reveal").performScrollTo().performClick()
        compose.onNodeWithTag("generated-password").assertRenderedText(copiedGenerator)
        document.await { application.deviceSettings.settings != null && compose.activity.hasWindowFocus() }
        compose.onNodeWithTag("generator-copy").performClick()
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == copiedGenerator }
        assertEquals(0, confirmations.get())
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("close-generator").performClick()
        document.waitAbsent("generated-password")
    }

    private data class LocalDocument(val state: DocumentState, val store: ViewModelStore, val entry: String, val path: String,
        val password: String = "PUBLIC local access master")

    private fun localDocument(owner: SecretAccessState, copy: (String) -> Boolean): LocalDocument {
        lateinit var local: DocumentState
        val store = ViewModelStore()
        compose.runOnIdle { local = DocumentState(application, NativeIconStageExecutor(application), owner); store.put("PUBLIC local access", local) }
        compose.runOnIdle { local.create("PUBLIC local access database", null, "PUBLIC local access master", newOperation()) }
        document.await { local.overview != null && !local.navigating && local.catalog.any { it.database == local.overview?.database } }
        compose.runOnIdle { local.beginMetadata(FormKind.NEW_GROUP, parent = null) }
        document.await { local.metadata != null && !local.navigating }
        compose.runOnIdle { local.editMetadata(name = "PUBLIC local access group"); local.saveNow() }
        document.await { local.activeForm == null && local.overview?.groups?.size == 1 }
        compose.runOnIdle { local.beginEntry() }
        document.await { local.entry != null && !local.navigating }
        compose.runOnIdle { local.edit(EntryTextField.TITLE, fields.title); local.edit(EntryTextField.PASSWORD, fields.password); local.saveNow() }
        document.await { local.activeForm == null && local.inspectedEntry != null && !local.navigating }
        var entry = ""
        var path = ""
        compose.runOnIdle { entry = local.inspectedEntry!!.id; path = local.catalog.single { it.database == local.overview?.database }.path }
        compose.activityRule.scenario.onActivity { activity -> activity.setContent {
            TaypeerTheme { LibraryScreen(local, rememberAttachmentActions(local), copySecret = copy, copy = {}) }
        } }
        document.waitFor("reveal-password")
        return LocalDocument(local, store, entry, path)
    }

    @Test fun acceptedDrawConfirmsAfterLockAndLateAckCannotResurrectTheScreen() {
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val completed = CountDownLatch(1)
        val time = AtomicLong()
        val owner = SecretAccessState(confirmer = AccessConfirmer { grant -> withContext(Dispatchers.IO) {
            started.countDown()
            check(release.await(20, TimeUnit.SECONDS))
            grant.confirm().also { time.set(it); completed.countDown() }
        } })
        val local = localDocument(owner) { false }
        val before = File(local.path).readBytes()
        try {
            compose.onNodeWithTag("reveal-password").performScrollTo().performClick()
            document.await { local.state.revealed == fields.password }
            compose.onNodeWithText(fields.password).performScrollTo().assertIsDisplayed()
            compose.onRoot().captureToImage()
            assertTrue(started.await(20, TimeUnit.SECONDS))
            compose.runOnIdle { local.state.lock(); assertNull(local.state.properties); assertTrue(local.state.revealed.isEmpty()) }
            release.countDown()
            assertTrue(completed.await(20, TimeUnit.SECONDS))
            document.await { owner.view.value.attempts.isEmpty() }
            compose.runOnIdle { assertNull(local.state.overview); assertNull(local.state.properties); assertNull(local.state.inspectedEntry); assertTrue(local.state.revealed.isEmpty()) }
            assertArrayEquals(before, File(local.path).readBytes())
            compose.runOnIdle { local.state.unlock(local.state.lockedCopy!!, local.password) }
            document.await { local.state.overview != null && !local.state.opening && !local.state.navigating }
            compose.runOnIdle { local.state.selectEntry(local.entry) }
            document.await { local.state.properties?.entry == local.entry && !local.state.navigating }
            compose.runOnIdle { assertEquals(time.get(), local.state.properties!!.accessedAt); local.state.showHistory(FormKind.ENTRY, local.entry) }
            document.await { local.state.history?.rows?.size == 1 }
            assertArrayEquals(before, File(local.path).readBytes())
        } finally { release.countDown(); compose.runOnIdle { local.store.clear() }; owner.close() }
    }

    @Test fun lostMetadataAckRetryDoesNotRevealOrCopyTheSecretAgain() {
        val calls = AtomicInteger()
        val copies = AtomicInteger()
        val owner = SecretAccessState(confirmer = AccessConfirmer { grant -> withContext(Dispatchers.IO) {
            val confirmed = grant.confirm()
            if (calls.incrementAndGet() == 1) throw AndroidException.CommitUncertain()
            confirmed
        } })
        val local = localDocument(owner) { value ->
            application.clipboard.copy(value, application.deviceSettings.settings!!.clipboardSeconds)
            copies.incrementAndGet()
            true
        }
        val before = File(local.path).readBytes()
        try {
            document.await { application.deviceSettings.settings != null && compose.activity.hasWindowFocus() }
            compose.onNodeWithTag("copy-password").performScrollTo().performClick()
            document.waitFor("access-metadata-error")
            compose.runOnIdle { assertNull(local.state.properties!!.accessedAt); assertTrue(local.state.revealed.isEmpty()) }
            assertEquals(1, copies.get())
            assertEquals(1, calls.get())
            assertArrayEquals(before, File(local.path).readBytes())
            compose.runOnIdle { local.state.lock() }
            document.waitFor("master-password")
            document.waitFor("access-metadata-error")
            compose.onNodeWithTag("retry-access").performScrollTo().performClick()
            document.await { calls.get() == 2 && owner.view.value.attempts.isEmpty() }
            assertEquals(2, calls.get())
            assertEquals(1, copies.get())
            compose.runOnIdle { assertNull(local.state.properties); assertNull(local.state.overview); assertTrue(local.state.revealed.isEmpty()) }
            compose.onNodeWithTag("master-password").performScrollTo().performTextReplacement(local.password)
            compose.onNodeWithTag("authenticate").performClick()
            document.await { local.state.overview != null && !local.state.opening && !local.state.navigating }
            compose.runOnIdle { local.state.selectEntry(local.entry) }
            document.await { local.state.properties?.entry == local.entry && !local.state.navigating }
            compose.runOnIdle { assertNotNull(local.state.properties!!.accessedAt) }
            compose.runOnIdle { assertTrue(local.state.revealed.isEmpty()); local.state.showHistory(FormKind.ENTRY, local.entry) }
            document.await { local.state.history?.rows?.size == 1 }
            assertArrayEquals(before, File(local.path).readBytes())
        } finally { compose.runOnIdle { local.store.clear() }; owner.close() }
    }

    @Test fun clipboardAcknowledgementSurvivesSynchronousHideAndComparisonLock() {
        val calls = AtomicInteger()
        val copies = AtomicInteger()
        val owner = SecretAccessState(confirmer = AccessConfirmer { grant -> withContext(Dispatchers.IO) {
            grant.confirm().also { calls.incrementAndGet() }
        } })
        var selected: DocumentState? = null
        val local = localDocument(owner) { value ->
            application.clipboard.copy(value, application.deviceSettings.settings!!.clipboardSeconds)
            if (copies.incrementAndGet() == 1) selected!!.hideSecrets() else selected!!.lock()
            true // The real clipboard acknowledged before the synchronous UI transition.
        }
        selected = local.state
        val before = File(local.path).readBytes()
        try {
            document.await { application.deviceSettings.settings != null && compose.activity.hasWindowFocus() }
            compose.onNodeWithTag("copy-password").performScrollTo().performClick()
            document.await { calls.get() == 1 && local.state.properties?.accessedAt != null }
            assertEquals(fields.password, clipboard.primaryClip?.getItemAt(0)?.text?.toString())
            compose.runOnIdle { assertTrue(local.state.revealed.isEmpty()); local.state.showHistory(FormKind.ENTRY, local.entry) }
            document.await { local.state.history?.rows?.size == 1 }
            var row: HistoryRow? = null
            compose.runOnIdle { row = local.state.history!!.rows.single() }
            compose.onNodeWithTag("history-version-${row!!.id}").performScrollTo().performClick()
            document.waitFor("saved-version-tab-0")
            compose.onNodeWithTag("compare-version").performScrollTo().performClick()
            document.await { local.state.comparison.saved != null && !local.state.comparison.busy }
            compose.onNodeWithTag("copy-comparison-saved").performScrollTo().performClick()
            document.await { calls.get() == 2 && owner.view.value.attempts.isEmpty() }
            assertEquals(2, copies.get())
            compose.runOnIdle { assertNull(local.state.overview); assertNull(local.state.properties); assertNull(local.state.historicalProperties); assertTrue(local.state.revealed.isEmpty()) }
            assertArrayEquals(before, File(local.path).readBytes())
            compose.runOnIdle { local.state.unlock(local.state.lockedCopy!!, local.password) }
            document.await { local.state.overview != null && !local.state.opening && !local.state.navigating }
            compose.runOnIdle { local.state.selectEntry(local.entry) }
            document.await { local.state.properties?.entry == local.entry && !local.state.navigating }
            compose.runOnIdle { assertNotNull(local.state.properties!!.accessedAt); assertTrue(local.state.revealed.isEmpty()); local.state.showHistory(FormKind.ENTRY, local.entry) }
            document.await { local.state.history?.rows?.size == 1 }
            assertArrayEquals(before, File(local.path).readBytes())
        } finally { compose.runOnIdle { local.store.clear() }; owner.close() }
    }
}
