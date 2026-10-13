package dev.taypeer

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.semantics.SemanticsActions
import dev.taypeer.bridge.ColorEdit
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/** Actual history routes, Rust worker payloads and durable ciphertext; all credentials are PUBLIC. */
class HistoryUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val saved = PublicEntryFields(
        title = "PUBLIC history saved title",
        username = "PUBLIC history saved username",
        password = "PUBLIC history saved password",
        url = "https://example.invalid/history/saved",
        notes = "PUBLIC history saved notes",
        tags = listOf("PUBLIC history saved tag"),
    )
    private val current = PublicEntryFields(
        title = "PUBLIC history current title",
        username = "PUBLIC history current username",
        password = "PUBLIC history current password",
        url = "https://example.invalid/history/current",
        notes = "PUBLIC history current notes",
        tags = listOf("PUBLIC history current tag"),
    )
    private val clipboard get() = compose.activity.getSystemService(ClipboardManager::class.java)

    @Before fun createPublicDocument() {
        document = PublicDocumentUiFixture(compose)
        document.create()
    }

    @After fun clearOnlyThisTestsPublicClipboardAndRestorePreferences() {
        if (!::document.isInitialized) return
        compose.runOnIdle {
            val clip = clipboard.primaryClip
            if (clip?.itemCount == 1 && clip.getItemAt(0).text?.toString() in setOf(saved.password, current.password)) {
                clipboard.clearPrimaryClip()
            }
        }
        document.restorePreferences()
    }

    private fun openSavedVersion(original: String): String {
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == 2 }
        var revision = ""
        compose.runOnIdle {
            revision = document.state.entryHistory.single { it.name == saved.title }.id
            assertEquals(setOf(saved.title, current.title), document.state.entryHistory.map { it.name }.toSet())
        }
        compose.onNodeWithTag("history-version-$revision").performScrollTo().performClick()
        compose.runOnIdle { assertNull(document.state.history) }
        compose.onNodeWithTag("history-view-version").performScrollTo().performClick()
        document.waitFor("saved-version-screen")
        document.await {
            document.state.history?.target == original && document.state.historicalRevision == revision &&
                document.state.historicalEntry?.title == saved.title && !document.state.navigating
        }
        return revision
    }

    private fun openComparison() {
        compose.onNodeWithTag("compare-version").performScrollTo().performClick()
        document.waitFor("comparison-screen")
        document.await {
            document.state.comparison.showing && !document.state.comparison.busy &&
                document.state.comparison.saved?.title == saved.title && document.state.comparison.current?.title == current.title
        }
    }

    private fun assertBothComparisonPasswordsMasked() {
        compose.runOnIdle {
            assertNull(document.state.comparison.savedPassword)
            assertNull(document.state.comparison.currentPassword)
            assertEquals(true, document.state.comparison.saved?.hasPassword)
            assertEquals(true, document.state.comparison.current?.hasPassword)
        }
        compose.onNodeWithTag("comparison-saved-password").assertTextContains("••••••••••••")
        compose.onNodeWithTag("comparison-current-password").assertTextContains("••••••••••••")
        compose.onNodeWithText(saved.password).assertDoesNotExist()
        compose.onNodeWithText(current.password).assertDoesNotExist()
    }

    private fun copyComparison(tag: String, expected: String) {
        // A fresh PUBLIC sentinel prevents a missing callback passing with a previous clip.
        compose.runOnIdle {
            clipboard.setPrimaryClip(ClipData.newPlainText("PUBLIC history UI sentinel", "PUBLIC history clipboard sentinel"))
        }
        compose.onNodeWithTag(tag).performScrollTo().performClick()
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == expected && !document.state.comparison.busy }
        compose.runOnIdle {
            assertEquals(expected, clipboard.primaryClip?.getItemAt(0)?.text?.toString())
            assertEquals(true, clipboard.primaryClip?.description?.extras?.getBoolean("android.content.extra.IS_SENSITIVE"))
        }
    }

    private fun closeComparison() {
        compose.onNodeWithTag("navigation-back").performClick()
        document.waitFor("saved-version-screen")
        document.await { !document.state.comparison.showing && !document.state.comparison.busy }
        compose.runOnIdle {
            assertNull(document.state.comparison.savedPassword)
            assertNull(document.state.comparison.currentPassword)
            assertNull(document.state.comparison.saved)
            assertNull(document.state.comparison.current)
        }
        compose.onNodeWithText(saved.password).assertDoesNotExist()
        compose.onNodeWithText(current.password).assertDoesNotExist()
    }

    @Test fun savedAndCurrentPasswordActionsAreIndependentReadOnlyAndHiddenAfterBack() {
        val group = document.group("PUBLIC history comparison group")
        val original = document.entry(group, saved)
        document.update(original, current)
        val before = document.savedBytes()
        val revision = openSavedVersion(original)
        openComparison()
        assertBothComparisonPasswordsMasked()

        copyComparison("copy-comparison-saved", saved.password)
        assertBothComparisonPasswordsMasked()
        copyComparison("copy-comparison-current", current.password)
        assertBothComparisonPasswordsMasked()

        compose.onNodeWithTag("reveal-comparison-saved").performScrollTo().performClick()
        document.await { document.state.comparison.savedPassword == saved.password && !document.state.comparison.busy }
        compose.onNodeWithTag("comparison-saved-password").assertTextContains(saved.password)
        compose.onNodeWithTag("comparison-current-password").assertTextContains("••••••••••••")
        compose.runOnIdle { assertNull(document.state.comparison.currentPassword) }

        compose.onNodeWithTag("reveal-comparison-current").performScrollTo().performClick()
        document.await { document.state.comparison.currentPassword == current.password && !document.state.comparison.busy }
        compose.onNodeWithTag("comparison-current-password").assertTextContains(current.password)
        compose.onNodeWithTag("comparison-saved-password").assertTextContains(saved.password)
        compose.onNodeWithTag("reveal-comparison-saved").performScrollTo().performClick()
        compose.runOnIdle {
            assertNull(document.state.comparison.savedPassword)
            assertEquals(current.password, document.state.comparison.currentPassword)
        }
        compose.onNodeWithTag("comparison-saved-password").assertTextContains("••••••••••••")
        compose.onNodeWithTag("comparison-current-password").assertTextContains(current.password)

        assertArrayEquals(before, document.savedBytes())
        closeComparison()
        compose.runOnIdle {
            assertEquals(revision, document.state.historicalRevision)
            assertEquals(2, document.state.history?.rows?.size)
            assertNull(document.state.activeForm)
        }
        openComparison()
        assertBothComparisonPasswordsMasked()
        closeComparison()
        assertArrayEquals(before, document.savedBytes())
    }

    @Test fun restoringSavedVersionDefaultsToActiveSavedGroupAndRetainsIdentityAndEarlierHistory() {
        val source = document.group("PUBLIC history restore source")
        val destinationName = "PUBLIC history restore destination"
        val destination = document.group(destinationName)
        val original = document.entry(source, saved)
        document.update(original, current)
        val before = document.savedBytes()
        openSavedVersion(original)
        var previousRevisions = emptySet<String>()
        compose.runOnIdle { previousRevisions = document.state.history!!.rows.map { it.id }.toSet() }

        compose.onNodeWithTag("restore-version").performScrollTo().performClick()
        document.waitFor("restore-version-screen")
        document.await { compose.onAllNodes(hasTestTag("confirm-restore-version") and isEnabled()).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("history-destination").assertTextContains("PUBLIC history restore source")
        compose.onNodeWithTag("history-destination").performScrollTo().performClick()
        compose.onNodeWithTag("history-destination-$destination").performClick()
        compose.onNodeWithTag("history-destination").assertTextContains(destinationName)
        compose.onNodeWithTag("confirm-restore-version").assertIsEnabled()
        assertArrayEquals(before, document.savedBytes())
        compose.runOnIdle { assertEquals(listOf(original), document.state.overview!!.entries.map { it.id }) }

        compose.onNodeWithTag("confirm-restore-version").performClick()
        document.await {
            document.state.history == null && !document.state.navigating && document.state.overview?.entries?.size == 1 &&
                document.state.inspectedEntry?.let { it.id == original && it.group == destination && it.title == saved.title } == true
        }
        compose.runOnIdle {
            val restored = document.state.overview!!.entries.single()
            assertEquals(original, restored.id)
            assertEquals(destination, restored.group)
            assertEquals(saved.title, restored.title)
            assertEquals(saved.username, restored.username)
        }
        assertFalse(before.contentEquals(document.savedBytes()))
        document.inspect(destination, original)
        assertInspectedFields(original, saved)
        assertRetainedHistory(previousRevisions)

        document.reopen()
        document.inspect(destination, original)
        assertInspectedFields(original, saved)
        assertRetainedHistory(previousRevisions)
        compose.runOnIdle { assertEquals(1, document.state.overview!!.entries.size) }
    }

    @Test fun savedVersionTabsReadExactHistoricalPropertiesAndMaskSecretsWithoutWriting() {
        val group = document.group("PUBLIC historical properties group")
        val expiration = 1_794_580_920_000L
        val savedForeground = 0xA1B2C3FFu
        val savedBackground = 0x1A2B3CFFu
        val ordinaryName = "PUBLIC historical ordinary attribute"
        val protectedName = "PUBLIC historical protected attribute"
        val savedOrdinary = "PUBLIC saved attribute value"
        val savedProtected = "PUBLIC saved protected attribute value"
        fun expectedTime(milliseconds: Long): String = java.text.SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS 'UTC'", java.util.Locale.ROOT)
            .apply { timeZone = java.util.TimeZone.getTimeZone("UTC") }.format(java.util.Date(milliseconds))
        fun fill(fields: PublicEntryFields) {
            document.state.edit(EntryTextField.TITLE, fields.title)
            document.state.edit(EntryTextField.USERNAME, fields.username)
            document.state.edit(EntryTextField.PASSWORD, fields.password)
            document.state.edit(EntryTextField.URL, fields.url)
            document.state.edit(EntryTextField.NOTES, fields.notes)
            document.state.editTags(fields.tags.joinToString("\n"))
        }
        compose.runOnIdle { document.state.chooseGroup(group) }
        document.await { document.state.selectedGroup == group && !document.state.navigating }
        compose.runOnIdle { document.state.beginEntry() }
        document.await { document.state.entry != null && !document.state.navigating }
        compose.runOnIdle {
            fill(saved)
            document.state.editExpiry(expiration.toString())
            document.state.enableExpiry(true)
            document.state.setIcon("server")
            document.state.setColors(ColorEdit.Set(savedForeground), ColorEdit.Set(savedBackground))
            document.state.editAttribute(null, ordinaryName, savedOrdinary, false)
            document.state.editAttribute(null, protectedName, savedProtected, true)
            document.state.saveNow()
        }
        document.await { document.state.entry == null && document.state.inspectedEntry?.title == saved.title && !document.state.navigating }
        var original = ""
        compose.runOnIdle { original = document.state.inspectedEntry!!.id; document.state.beginEntry(original) }
        document.await { document.state.entry?.source?.entry == original && !document.state.navigating }
        compose.runOnIdle {
            val attributes = document.state.entry!!.source.attributes
            fill(current)
            document.state.enableExpiry(false)
            document.state.setIcon("heart")
            document.state.setColors(ColorEdit.Set(0xC3B2A1FFu), ColorEdit.Set(0x3C2B1AFFu))
            document.state.editAttribute(attributes.single { it.name == ordinaryName }.id, ordinaryName, "PUBLIC current attribute value", false)
            document.state.editAttribute(attributes.single { it.name == protectedName }.id, protectedName, "PUBLIC current protected attribute value", true)
            document.state.saveNow()
        }
        document.await { document.state.entry == null && document.state.inspectedEntry?.title == current.title && !document.state.navigating }
        compose.runOnIdle {
            assertEquals(current.tags.toSet(), document.state.properties!!.tags.toSet())
            assertNull(document.state.properties!!.expiresAt)
            assertEquals("heart", document.state.properties!!.appearance.lucide)
        }
        val before = document.savedBytes()
        val revision = openSavedVersion(original)
        document.await { document.state.historicalProperties != null }
        var protectedId = ""
        var createdAt = 0L
        var modifiedAt = 0L
        compose.runOnIdle {
            val properties = document.state.historicalProperties!!
            assertEquals(original, properties.entry)
            assertEquals(saved.tags.toSet(), properties.tags.toSet())
            assertEquals(expiration, properties.expiresAt)
            assertEquals("server", properties.appearance.lucide)
            assertEquals(savedForeground, properties.appearance.foreground)
            assertEquals(savedBackground, properties.appearance.background)
            assertTrue(properties.createdAt > 0)
            assertTrue(properties.modifiedAt >= properties.createdAt)
            createdAt = properties.createdAt; modifiedAt = properties.modifiedAt
            protectedId = document.state.historicalAttributes.single { it.name == protectedName }.id!!
        }
        compose.onNodeWithText(saved.tags.joinToString(", ")).assertExists()
        compose.onNodeWithText(expectedTime(expiration)).assertExists()
        compose.onNodeWithTag("reveal-password").performScrollTo().performClick()
        document.await { document.state.revealed == saved.password }

        compose.onNodeWithTag("entry-tab-1").performScrollTo().performClick()
        document.waitFor("saved-version-tab-1")
        compose.runOnIdle { assertTrue(document.state.revealed.isEmpty()); assertNull(document.state.activeForm) }
        compose.onNodeWithText(savedOrdinary).assertExists()
        compose.onNodeWithText("PUBLIC current attribute value").assertDoesNotExist()
        compose.onNodeWithText(savedProtected).assertDoesNotExist()
        compose.onNodeWithTag("reveal-saved-attribute-$protectedId").performScrollTo().performClick()
        document.await { document.state.revealed == savedProtected && document.state.revealedAttribute == protectedId }
        compose.onNodeWithText(savedProtected).assertExists()
        compose.onNodeWithTag("add-attribute").assertDoesNotExist()
        compose.onNodeWithTag("add-attachment").assertDoesNotExist()

        compose.onNodeWithTag("entry-tab-2").performScrollTo().performClick()
        document.waitFor("saved-version-tab-2")
        compose.runOnIdle { assertTrue(document.state.revealed.isEmpty()); assertNull(document.state.revealedAttribute) }
        compose.onNodeWithText(savedProtected).assertDoesNotExist()
        compose.onNodeWithText("a1b2c3ff").assertExists()
        compose.onNodeWithText("1a2b3cff").assertExists()
        compose.onNodeWithTag("appearance-foreground").assert(SemanticsMatcher.keyNotDefined(SemanticsActions.SetText))
        compose.onNodeWithTag("icon-picker").assertDoesNotExist()

        compose.onNodeWithTag("entry-tab-3").performScrollTo().performClick()
        document.waitFor("saved-version-tab-3")
        compose.onNodeWithText(original).assertExists()
        compose.runOnIdle { clipboard.setPrimaryClip(ClipData.newPlainText("PUBLIC property sentinel", "PUBLIC sentinel")) }
        compose.onNodeWithTag("copy-property-id").performScrollTo().performClick()
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == original }
        compose.onNodeWithTag("copy-property-modified").performScrollTo().performClick()
        document.await { clipboard.primaryClip?.getItemAt(0)?.text?.toString() == expectedTime(modifiedAt) }
        compose.onAllNodesWithText(expectedTime(createdAt)).assertCountEquals(if (expectedTime(createdAt) == expectedTime(modifiedAt)) 2 else 1)
        compose.onNodeWithText(expectedTime(expiration)).assertExists()
        compose.onNodeWithText(saved.tags.joinToString(", ")).assertExists()
        compose.onNodeWithTag("save-form").assertDoesNotExist()
        compose.onNodeWithTag("edit-entry").assertDoesNotExist()

        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.waitFor("saved-version-tab-4")
        compose.onNodeWithTag("compare-version").assertDoesNotExist()
        compose.onNodeWithTag("restore-version").assertDoesNotExist()
        compose.onNodeWithTag("history-compare-version").assertExists()
        compose.onNodeWithTag("history-restore-version").assertExists()
        compose.onNodeWithTag("history-version-$revision").assertExists()
        compose.runOnIdle { assertEquals(2, document.state.history!!.rows.size); assertNull(document.state.activeForm) }
        var latest = ""
        compose.runOnIdle { latest = document.state.history!!.rows.single { it.name == current.title }.id }
        compose.onNodeWithTag("history-version-$latest").performScrollTo().performClick()
        compose.runOnIdle { assertEquals(revision, document.state.historicalRevision) }
        compose.onNodeWithTag("history-view-version").performScrollTo().performClick()
        document.await {
            document.state.historicalRevision == latest && document.state.historicalEntry?.title == current.title &&
                document.state.historicalProperties?.appearance?.lucide == "heart" && document.state.historicalTab == 0
        }
        compose.onNodeWithTag("saved-version-tab-0").assertExists()
        compose.onNodeWithText(current.tags.joinToString(", ")).assertExists()
        compose.onNodeWithText(saved.password).assertDoesNotExist()
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("compare-version").performScrollTo().assertIsEnabled()
        compose.onNodeWithTag("restore-version").performScrollTo().assertIsEnabled()
    }

    @Test fun restoringVersionWithDeletedSavedGroupRequiresAnExplicitActiveDestination() {
        val source = document.group("PUBLIC deleted saved group")
        val destination = document.group("PUBLIC active restore group")
        val original = document.entry(source, saved)
        document.update(original, current)
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == 2 }
        var revision = ""
        compose.runOnIdle {
            revision = document.state.entryHistory.single { it.name == saved.title }.id
            document.state.lifecycle.openAction(ObjectActionSelection(FormKind.ENTRY, original, ObjectAction.MOVE, current.title, source))
        }
        document.await { document.state.lifecycle.selection != null && !document.state.lifecycle.busy }
        compose.runOnIdle { document.state.lifecycle.confirmAction(destination, current.title, newOperation()) }
        document.await { document.state.lifecycle.selection == null && !document.state.lifecycle.busy && document.state.overview?.entries?.single()?.group == destination }
        compose.runOnIdle { document.state.trash(FormKind.GROUP, source, null) }
        document.await { !document.state.navigating && document.state.overview?.groups?.none { it.id == source } == true }
        document.inspect(destination, original)
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.any { it.id == revision } }
        compose.onNodeWithTag("history-version-$revision").performScrollTo().performClick()
        compose.onNodeWithTag("history-view-version").performScrollTo().performClick()
        document.waitFor("saved-version-screen")
        document.await { document.state.historicalEntry?.group == source && document.state.historicalProperties != null }
        var versions = 0
        compose.runOnIdle { versions = document.state.history!!.rows.size }
        val before = document.savedBytes()
        compose.onNodeWithTag("restore-version").performScrollTo().performClick()
        document.waitFor("restore-version-screen")
        compose.onNodeWithTag("confirm-restore-version").assertIsNotEnabled()
        compose.onNodeWithTag("history-destination").assertTextContains(document.text(R.string.choose_group))
        compose.onNodeWithTag("history-destination").performScrollTo().performClick()
        compose.onNodeWithTag("history-destination-$destination").performClick()
        assertArrayEquals(before, document.savedBytes())
        compose.onNodeWithTag("confirm-restore-version").performClick()
        document.await { document.state.history == null && !document.state.navigating && document.state.inspectedEntry?.title == saved.title }
        compose.runOnIdle {
            assertEquals(original, document.state.inspectedEntry!!.id)
            assertEquals(destination, document.state.inspectedEntry!!.group)
            assertEquals(1, document.state.overview!!.entries.size)
        }
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == versions + 1 }
        assertFalse(before.contentEquals(document.savedBytes()))
    }

    @Test fun selectedHistoryCommandsDoNotSurviveStayOrBackAndExplicitCompareIsReadOnly() {
        val group = document.group("PUBLIC guarded history group")
        val original = document.entry(group, saved)
        document.update(original, current)
        val before = document.savedBytes()
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == 2 }
        compose.onNodeWithTag("history-view-version").assertIsNotEnabled()
        compose.onNodeWithTag("history-compare-version").assertIsNotEnabled()
        compose.onNodeWithTag("history-restore-version").assertIsNotEnabled()
        compose.onNodeWithTag("history-actions").performClick()
        compose.onNodeWithTag("history-clear-version").assertIsNotEnabled()
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
            .sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        var revision = ""
        compose.runOnIdle { revision = document.state.entryHistory.single { it.name == saved.title }.id }
        compose.onNodeWithTag("history-version-$revision").performScrollTo().performClick()
        assertArrayEquals(before, document.savedBytes())

        compose.onNodeWithTag("edit-entry").performClick()
        document.await { document.state.entry?.source?.entry == original && !document.state.navigating }
        compose.runOnIdle { document.state.edit(EntryTextField.TITLE, "PUBLIC unsaved guarded history title") }
        document.await { document.state.activeForm?.dirty == true && document.state.entryHistory.size == 2 }
        compose.onNodeWithTag("history-version-$revision").performScrollTo().performClick()
        compose.onNodeWithTag("history-compare-version").performScrollTo().performClick()
        document.waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-stay").performClick()
        document.waitAbsent("unsaved-changes")
        compose.runOnIdle { assertNull(document.state.history); assertFalse(document.state.comparison.showing) }

        compose.onNodeWithTag("history-restore-version").performScrollTo().performClick()
        document.waitFor("unsaved-changes")
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
            .sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        document.waitAbsent("unsaved-changes")
        compose.runOnIdle { assertNull(document.state.history); assertFalse(document.state.comparison.showing) }

        document.menu("open-entry-history")
        document.waitFor("unsaved-changes")
        compose.onNodeWithTag("navigation-discard").performClick()
        document.waitFor("history-list-screen")
        compose.runOnIdle { assertFalse(document.state.comparison.showing); assertNull(document.state.historicalRevision) }
        compose.onNodeWithTag("restore-version-screen").assertDoesNotExist()
        compose.onNodeWithTag("purge-selected-history").assertIsNotEnabled()
        compose.onNodeWithTag("history-version-$revision").performScrollTo().performClick()
        document.waitFor("saved-version-screen")
        document.await { document.state.historicalEntry?.title == saved.title }
        openComparison()
        assertBothComparisonPasswordsMasked()
        assertArrayEquals(before, document.savedBytes())
        closeComparison()
        assertArrayEquals(before, document.savedBytes())
    }

    private fun assertInspectedFields(entry: String, fields: PublicEntryFields) {
        compose.onNodeWithTag("entry-tab-0").performScrollTo().performClick()
        document.await { document.state.inspectedEntry?.id == entry && document.state.properties?.entry == entry }
        compose.runOnIdle {
            val view = document.state.inspectedEntry!!
            assertEquals(fields.title, view.title)
            assertEquals(fields.username, view.username)
            assertEquals(fields.url, view.url)
            assertEquals(fields.notes, view.notes)
            assertEquals(fields.tags.toSet(), document.state.properties!!.tags.toSet())
        }
        compose.onNodeWithText(fields.username).assertExists()
        compose.onNodeWithTag("reveal-password").performScrollTo().performClick()
        document.await { document.state.revealed == fields.password }
        compose.onNodeWithText(fields.password).assertExists()
    }

    private fun assertRetainedHistory(previousRevisions: Set<String>) {
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { document.state.entryHistory.size == 3 }
        compose.runOnIdle {
            assertEquals(setOf(saved.title, current.title), document.state.entryHistory.map { it.name }.toSet())
            assertTrue(document.state.entryHistory.map { it.id }.toSet().containsAll(previousRevisions))
        }
        compose.onNodeWithTag("entry-tab-0").performScrollTo().performClick()
    }
}
