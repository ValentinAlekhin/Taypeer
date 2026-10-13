package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import org.junit.After
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Genuine group context action and Activity lifecycle, without implicit icon acquisition. */
class GroupFaviconsNavigationUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val document by lazy { PublicDocumentUiFixture(compose) }
    private val state get() = document.state

    @After fun restorePreferences() {
        compose.runOnIdle { state.groupFavicons.close(); state.lock() }
        document.restorePreferences()
    }

    @Test fun actualGroupMenuOpensDefaultFormWithoutHttpAndBackgroundRevokesItAfterRecreation() {
        document.create()
        val group = document.group("PUBLIC group favicon context")
        val image = """<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#14dc28"/></svg>""".toByteArray()
        PublicIconServer(image).use { server ->
            document.entry(group, PublicEntryFields(title = "PUBLIC group favicon context entry", url = server.url))
            document.showGroup(group)
            val before = document.savedBytes()
            compose.onNodeWithTag("group-menu").performClick()
            document.waitFor("database-drawer-open")
            compose.onNodeWithTag("select-group-$group").performScrollTo().assertIsDisplayed().performTouchInput { longClick() }
            document.waitFor("group-context-menu")
            document.waitFor("database-drawer-closed")
            compose.onNodeWithTag("download-group-favicons").performScrollTo().assertIsEnabled().performClick()
            document.waitFor("bulk-favicon-screen")
            document.waitAbsent("group-context-menu")
            document.await { !state.navigating }
            compose.onNodeWithTag("bulk-favicon-recursive").assertIsOff()
            compose.onNodeWithTag("bulk-favicon-replace").assertIsOff()
            compose.runOnIdle { assertFalse(state.groupFavicons.started); assertTrue(state.groupFavicons.rows.isEmpty()) }
            assertEquals(0, server.requests.get())
            assertArrayEquals(before, document.savedBytes())

            compose.activityRule.scenario.recreate()
            document.waitFor("bulk-favicon-screen")
            compose.runOnIdle { assertFalse(state.groupFavicons.started); assertEquals(group, state.groupFavicons.group!!.id) }
            compose.onNodeWithTag("load-group-favicons").assertIsEnabled()
            assertEquals(0, server.requests.get())
            assertArrayEquals(before, document.savedBytes())

            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            document.waitFor("master-password")
            compose.onNodeWithTag("bulk-favicon-screen").assertDoesNotExist()
            compose.runOnIdle {
                assertFalse(state.groupFavicons.showing)
                assertNull(state.groupFavicons.group)
                assertTrue(state.groupFavicons.rows.isEmpty())
                assertNull(state.overview)
                assertTrue(state.revealed.isEmpty())
            }
            assertEquals(0, server.requests.get())
            assertArrayEquals(before, document.savedBytes())
        }
    }
}
