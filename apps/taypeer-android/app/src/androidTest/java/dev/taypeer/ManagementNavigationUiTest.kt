package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*

/** The actual Activity preserves Save/Discard/Stay and never invents a remote recipient. */
class ManagementNavigationUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private var group = ""
    private var entry = ""
    private val state get() = document.state
    @Before fun prepare() {
        document = PublicDocumentUiFixture(compose)
        document.create()
        group = document.group("PUBLIC management navigation group")
        entry = document.entry(group, PublicEntryFields(title = "PUBLIC saved management navigation title"))
    }
    @After fun restorePreferences() {
        if (::document.isInitialized) { compose.runOnIdle { state.lock() }; document.restorePreferences() }
    }

    @Test fun stayCancelsTheDeferredTransferAndDiscardOpensOnlyAnExplicitEmptySelector() {
        val before = document.savedBytes()
        compose.onNodeWithTag("edit-entry").performClick()
        document.waitFor("entry-title")
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC unsaved management navigation title")
        compose.runOnIdle { state.beginManagementTransfer() }
        document.waitFor("navigation-stay")
        compose.onNodeWithTag("navigation-stay").performClick()
        document.waitAbsent("navigation-stay")
        compose.onNodeWithTag("entry-title").assertTextEquals("PUBLIC unsaved management navigation title")
        compose.runOnIdle { assertNull(state.management.route); assertFalse(state.management.preparing) }
        assertArrayEquals(before, document.savedBytes())
        compose.runOnIdle { state.beginManagementTransfer() }
        document.waitFor("navigation-discard")
        compose.onNodeWithTag("navigation-discard").performClick()
        document.waitFor("transfer-management-screen")
        compose.onNodeWithTag("confirm-management-transfer").assertIsNotEnabled()
        compose.onNodeWithTag("management-peer-observation").assertDoesNotExist()
        var firstOperation = ""
        compose.runOnIdle {
            assertTrue(state.management.peers.isEmpty()); assertTrue(state.management.rows.isEmpty())
            firstOperation = (state.management.route as ManagementRoute.Transfer).operation
        }
        compose.onNodeWithTag("cancel-form").performClick()
        document.waitAbsent("transfer-management-screen")
        compose.runOnIdle { assertNull(state.management.route); assertNull(state.entry) }
        assertArrayEquals(before, document.savedBytes())
        // An independent later opening cannot revive the dirty-navigation attempt.
        document.waitFor("group-menu")
        compose.runOnIdle { state.beginManagementTransfer() }
        document.waitFor("transfer-management-screen")
        compose.onNodeWithTag("confirm-management-transfer").assertIsNotEnabled()
        compose.runOnIdle {
            assertTrue(state.management.rows.isEmpty())
            assertNotEquals(firstOperation, (state.management.route as ManagementRoute.Transfer).operation)
        }
    }

    @Test fun saveCommitsTheDraftOnceBeforeOpeningTheTransferForm() {
        val before = document.savedBytes()
        compose.onNodeWithTag("edit-entry").performClick()
        document.waitFor("entry-title")
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC saved once before management transfer")
        compose.runOnIdle { state.beginManagementTransfer() }
        document.waitFor("navigation-save")
        compose.onNodeWithTag("navigation-save").performClick()
        document.waitFor("transfer-management-screen")
        val saved = document.savedBytes()
        assertFalse(before.contentEquals(saved))
        compose.onNodeWithTag("confirm-management-transfer").assertIsNotEnabled()
        compose.onNodeWithTag("cancel-form").performClick()
        document.waitAbsent("transfer-management-screen")
        assertArrayEquals(saved, document.savedBytes())
        compose.runOnIdle { assertTrue(state.management.rows.isEmpty()); assertNull(state.activeForm); assertNull(state.entry) }
        document.inspect(group, entry)
        compose.onNode(hasText("PUBLIC saved once before management transfer") and
            hasAnyAncestor(hasTestTag("route-inspected"))).performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { state.entryHistory.size == 2 }
    }
}
