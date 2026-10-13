package dev.taypeer

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.FormKind
import dev.taypeer.platform.ExchangeScheduler
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.UUID

/** Screen departures exercise the actual Compose handlers and supervised Rust document. */
class AndroidNavigationTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val password = "PUBLIC navigation master password"
    private val persistedTitle = "PUBLIC navigation persisted entry"
    private var database = ""
    private var originalTheme: ThemeMode? = null
    private var originalLanguage: UiLanguage? = null
    private val application get() = compose.activity.application as TaypeerApplication

    @Before fun establishPublicEnvironment() {
        compose.runOnIdle {
            originalTheme = application.uiPreferences.theme
            originalLanguage = application.uiPreferences.language
            application.uiPreferences.selectTheme(ThemeMode.SYSTEM)
            application.uiPreferences.selectLanguage(UiLanguage.ENGLISH)
        }
        if (Build.VERSION.SDK_INT >= 33 && compose.activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(compose.activity.packageName, Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    @After fun releasePublicEnvironment() {
        ExchangeScheduler.stopForeground(compose.activity)
        ExchangeScheduler.cancel(compose.activity)
        val host = runBlocking { application.host.await() }
        compose.waitUntil(10_000) { !host.exchangeView(null).running }
        compose.runOnIdle {
            originalTheme?.let(application.uiPreferences::selectTheme)
            originalLanguage?.let(application.uiPreferences::selectLanguage)
        }
    }

    private fun waitFor(tag: String) {
        try {
            compose.waitUntil(20_000) { compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty() }
        } catch (timeout: ComposeTimeoutException) {
            var flags = ""
            compose.runOnIdle {
                compose.activity.documentState.let { state ->
                    flags = "overview=${state.overview != null}, entry=${state.entry != null}, metadata=${state.metadata != null}, form=${state.activeForm != null}, formKind=${state.activeForm?.kind}, formDirty=${state.activeForm?.dirty}, saveLabel=${state.saveLabel}, opening=${state.opening}, navigating=${state.navigating}, decision=${state.navigationDecision}, invalid=${state.invalidInput}, failure=${state.error}, history=${state.history != null}, exchange=${state.exchange != null}"
                }
            }
            throw AssertionError("Missing control $tag; $flags", timeout)
        }
    }

    private fun waitAbsent(tag: String) = compose.waitUntil(20_000) {
        compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isEmpty()
    }

    private fun createGroup() {
        if (compose.onAllNodes(hasTestTag("choose-another-database")).fetchSemanticsNodes().isNotEmpty()) {
            compose.onNodeWithTag("choose-another-database").performClick()
        }
        compose.onNodeWithTag("create-database").performClick()
        compose.onNodeWithTag("database-name").performTextReplacement("PUBLIC navigation ${UUID.randomUUID().toString().take(8)}")
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.onNodeWithTag("repeat-master-password").performTextReplacement(password)
        compose.onNodeWithTag("authenticate").performClick()
        waitFor("add-group")
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            state.catalog.any { it.database == state.overview?.database }
        }
        compose.runOnIdle { database = compose.activity.documentState.overview!!.database }
        compose.onNodeWithTag("add-group").performScrollTo().assertIsEnabled().performClick()
        waitFor("metadata-name")
        // The form is composed before its navigation/loading commands finish.
        // Preparation must wait for editable controls, then the acknowledged
        // native input revision rather than treating mere node presence as ready.
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            !state.editingBlocked && state.metadata != null && state.activeForm?.kind == FormKind.NEW_GROUP
        }
        var groupDraft = ""
        var initialRevision = 0uL
        compose.runOnIdle {
            compose.activity.documentState.activeForm!!.let { form ->
                groupDraft = form.draft
                initialRevision = form.revision
            }
        }
        compose.onNodeWithTag("metadata-name").assertIsEnabled().performTextReplacement("PUBLIC navigation group")
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            state.metadata?.name == "PUBLIC navigation group" && !state.editingBlocked &&
                state.activeForm?.let { it.draft == groupDraft && it.dirty && it.revision > initialRevision } == true
        }
        compose.onNodeWithTag("metadata-name").assertTextEquals("PUBLIC navigation group")
        compose.runOnIdle {
            compose.activity.getSystemService(android.view.inputmethod.InputMethodManager::class.java)
                .hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0)
        }
        compose.waitUntil(10_000) {
            compose.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) != true
        }
        compose.onNodeWithTag("save-form").assertIsEnabled().assertIsDisplayed().performTouchInput { click() }
        waitFor("add-entry")
        compose.onNodeWithTag("add-entry").assertIsEnabled()
        compose.runOnIdle {
            val state = compose.activity.documentState
            assertNull(state.metadata)
            assertNull(state.activeForm)
            assertTrue(state.overview!!.groups.any { it.name == "PUBLIC navigation group" })
        }
    }

    private fun addEntry() {
        compose.onNodeWithTag("add-entry").performClick()
        waitFor("entry-title")
    }

    private fun createSavedEntry() {
        createGroup()
        addEntry()
        compose.onNodeWithTag("entry-title").performTextReplacement(persistedTitle)
        compose.onNodeWithTag("save-form").performClick()
        waitFor("edit-entry")
        compose.onNodeWithTag("save-form").assertDoesNotExist()
    }

    private fun editEntry() {
        compose.onNodeWithTag("edit-entry").performClick()
        waitFor("entry-title")
    }

    private fun acknowledgeInput() = compose.waitUntil(20_000) {
        val state = compose.activity.documentState
        state.activeForm?.dirty == true && !state.navigating
    }

    private fun activeDraft(): String {
        var draft = ""
        compose.runOnIdle { draft = compose.activity.documentState.activeForm!!.draft }
        return draft
    }

    private fun savedFile(): File {
        var path = ""
        compose.runOnIdle { path = compose.activity.documentState.catalog.single { it.database == database }.path }
        return File(path)
    }

    private fun requestDeparture(control: String) {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag(control).performClick()
        waitFor("unsaved-changes")
    }

    private fun stay() {
        compose.onNodeWithTag("navigation-stay").performClick()
        waitAbsent("unsaved-changes")
    }

    private fun reopen() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("lock-database").performClick()
        waitFor("master-password")
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.tapWorkingCopyUnlock(database)
        waitFor("entry-search")
    }

    @Test fun settingsOffersStayAndDiscardWithoutSavingTheEntry() {
        createSavedEntry()
        val before = savedFile().readBytes()
        editEntry()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC navigation rejected settings edit")
        acknowledgeInput()
        val draft = activeDraft()

        requestDeparture("open-settings")
        compose.onNodeWithTag("route-settings").assertDoesNotExist()
        assertArrayEquals(before, savedFile().readBytes())
        stay()
        compose.onNodeWithTag("entry-title").assertTextContains("PUBLIC navigation rejected settings edit")
        compose.runOnIdle { assertEquals(draft, compose.activity.documentState.activeForm!!.draft) }

        requestDeparture("open-settings")
        compose.onNodeWithTag("navigation-discard").performClick()
        waitFor("route-settings")
        compose.waitUntil(20_000) { compose.activity.documentState.forms.none { it.draft == draft } }
        compose.onNodeWithTag("entry-title").assertDoesNotExist()
        compose.onNodeWithTag("save-form").assertDoesNotExist()
        compose.runOnIdle {
            val state = compose.activity.documentState
            assertNull(state.activeForm)
            assertTrue(state.forms.none { it.draft == draft })
            assertEquals(persistedTitle, state.overview!!.entries.single().title)
        }
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("navigation-back").performClick()
        waitFor("add-entry")
        reopen()
        compose.onNodeWithText(persistedTitle).assertExists()
        compose.onNodeWithText("PUBLIC navigation rejected settings edit").assertDoesNotExist()
        assertArrayEquals(before, savedFile().readBytes())
    }

    @Test fun lockClearsComparisonAndPasswordDraftSynchronouslyBeforePolling() {
        createSavedEntry()
        var entry = ""
        compose.runOnIdle { entry = compose.activity.documentState.inspectedEntry!!.id }
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-entry-history").performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.history?.rows?.size == 1 }
        var revision = ""
        compose.runOnIdle { revision = compose.activity.documentState.history!!.rows.single().id }
        compose.onNodeWithTag("history-version-$revision").performScrollTo().performClick()
        waitFor("saved-version-screen")
        compose.waitUntil(20_000) { compose.activity.documentState.historicalEntry != null }
        compose.onNodeWithTag("compare-version").performScrollTo().performClick()
        waitFor("comparison-screen")
        compose.waitUntil(20_000) { !compose.activity.documentState.comparison.busy }
        compose.runOnIdle {
            val state = compose.activity.documentState
            assertTrue(state.comparison.showing)
            state.lock()
            assertNull(state.overview); assertNull(state.entry); assertNull(state.metadata)
            assertNull(state.inspectedEntry); assertNull(state.historicalEntry); assertNull(state.history)
            assertFalse(state.comparison.showing)
            assertNull(state.comparison.saved); assertNull(state.comparison.current)
            assertNull(state.comparison.savedPassword); assertNull(state.comparison.currentPassword)
        }
        waitFor("master-password")
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.tapWorkingCopyUnlock(database)
        waitFor("entry-search")
        compose.onNodeWithTag("entry-$entry").performClick()
        waitFor("edit-entry")
        editEntry()
        compose.onNodeWithTag("entry-password").performTextReplacement("PUBLIC navigation unsaved secret")
        acknowledgeInput()
        compose.runOnIdle {
            val state = compose.activity.documentState
            assertEquals("PUBLIC navigation unsaved secret", state.entry!!.password)
            state.lock()
            assertNull(state.overview); assertNull(state.entry); assertNull(state.activeForm)
            assertNull(state.historicalEntry); assertFalse(state.comparison.showing)
        }
    }

    @Test fun invalidNavigationSaveKeepsTheEditorAndPendingDestination() {
        createGroup()
        val before = savedFile().readBytes()
        addEntry()
        compose.onNodeWithTag("entry-username").performTextReplacement("PUBLIC navigation unfinished username")
        acknowledgeInput()
        val draft = activeDraft()

        requestDeparture("open-settings")
        compose.onNodeWithTag("navigation-save").performClick()
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            state.invalidInput && !state.navigating
        }
        compose.onNodeWithTag("unsaved-changes").assertExists()
        compose.onNodeWithTag("route-settings").assertDoesNotExist()
        compose.onNodeWithTag("entry-username").assertTextContains("PUBLIC navigation unfinished username")
        compose.runOnIdle {
            val state = compose.activity.documentState
            assertEquals(draft, state.activeForm!!.draft)
            assertTrue(state.navigationDecision)
            assertFalse(state.error)
            assertTrue(state.overview!!.entries.isEmpty())
        }
        assertArrayEquals(before, savedFile().readBytes())

        stay()
        compose.onNodeWithTag("entry-title").assertExists()
        requestDeparture("open-settings")
        compose.onNodeWithTag("navigation-discard").performClick()
        waitFor("route-settings")
        compose.waitUntil(20_000) { compose.activity.documentState.forms.none { it.draft == draft } }
        compose.runOnIdle {
            assertNull(compose.activity.documentState.activeForm)
            assertTrue(compose.activity.documentState.forms.none { it.draft == draft })
        }
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("navigation-back").performClick()
        waitFor("add-entry")
        reopen()
        compose.runOnIdle { assertTrue(compose.activity.documentState.overview!!.entries.isEmpty()) }
    }

    @Test fun historyAndExchangeWaitForTheUnsavedEntryDecision() {
        createSavedEntry()
        val before = savedFile().readBytes()
        editEntry()
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC navigation rejected history edit")
        acknowledgeInput()
        val historyDraft = activeDraft()

        requestDeparture("open-entry-history")
        compose.runOnIdle { assertNull(compose.activity.documentState.history) }
        stay()
        compose.onNodeWithTag("entry-title").assertTextContains("PUBLIC navigation rejected history edit")
        compose.runOnIdle { assertEquals(historyDraft, compose.activity.documentState.activeForm!!.draft) }
        assertArrayEquals(before, savedFile().readBytes())
        requestDeparture("open-entry-history")
        compose.onNodeWithTag("navigation-discard").performClick()
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            state.history != null && !state.navigating && state.forms.none { it.draft == historyDraft }
        }
        compose.onNodeWithTag("entry-title").assertDoesNotExist()
        compose.onNodeWithTag("save-form").assertDoesNotExist()
        compose.onNodeWithText(persistedTitle).assertExists()
        compose.onNodeWithText("PUBLIC navigation rejected history edit").assertDoesNotExist()
        compose.runOnIdle {
            val state = compose.activity.documentState
            assertNull(state.activeForm)
            assertTrue(state.forms.none { it.draft == historyDraft })
            assertEquals(1, state.history!!.rows.size)
        }
        assertArrayEquals(before, savedFile().readBytes())
        compose.onNodeWithTag("navigation-back").performClick()
        waitFor("add-entry")

        compose.onNodeWithText(persistedTitle).performClick()
        waitFor("edit-entry")
        editEntry()
        val acceptedTitle = "PUBLIC navigation saved before exchange"
        compose.onNodeWithTag("entry-title").performTextReplacement(acceptedTitle)
        acknowledgeInput()
        val exchangeDraft = activeDraft()
        requestDeparture("open-exchange")
        compose.runOnIdle { assertNull(compose.activity.documentState.exchange) }
        stay()
        compose.onNodeWithTag("entry-title").assertTextContains(acceptedTitle)
        compose.runOnIdle { assertEquals(exchangeDraft, compose.activity.documentState.activeForm!!.draft) }
        assertArrayEquals(before, savedFile().readBytes())

        requestDeparture("open-exchange")
        compose.onNodeWithTag("navigation-save").performClick()
        compose.waitUntil(20_000) {
            val state = compose.activity.documentState
            state.exchange != null && !state.navigating && state.overview?.entries?.singleOrNull()?.title == acceptedTitle && state.forms.none { it.draft == exchangeDraft }
        }
        compose.onNodeWithTag("entry-title").assertDoesNotExist()
        compose.onNodeWithTag("save-form").assertDoesNotExist()
        compose.runOnIdle {
            val state = compose.activity.documentState
            assertNull(state.activeForm)
            assertTrue(state.forms.none { it.draft == exchangeDraft })
        }
        assertFalse(before.contentEquals(savedFile().readBytes()))
        compose.onNodeWithText(compose.activity.getString(R.string.stop_exchange)).performScrollTo().performClick()
        val host = runBlocking { application.host.await() }
        compose.waitUntil(10_000) { !host.exchangeView(null).running }
        compose.onNodeWithTag("navigation-back").performClick()
        waitFor("add-entry")
        reopen()
        compose.onNodeWithText(acceptedTitle).assertExists()
        compose.onNodeWithText(acceptedTitle).performClick()
        waitFor("edit-entry")
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.activity.documentState.entryHistory.size == 2 }
        compose.onNodeWithText(persistedTitle).assertExists()
        compose.runOnIdle {
            assertEquals(setOf(persistedTitle, acceptedTitle), compose.activity.documentState.entryHistory.map { it.name }.toSet())
        }
    }
}
