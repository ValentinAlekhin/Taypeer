package dev.taypeer

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.KeystoreCredentials
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger

/** Actual form handlers, isolated Rust workers and explicit loopback HTTP; no fake success. */
class GroupFaviconsUiTest {
    @get:Rule val compose = createComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC group favicon UI master"
    private val image = """<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#14dc28"/></svg>""".toByteArray()

    @Test fun openingAndOptionsNeverFetchThenExplicitLoadDisplaysActualPartialRowsAndExactRetry() = fixture { session, manager ->
        val group = publicGroup(session)
        PublicFaviconServer(image).use { server ->
            val good = entry(session, group, "PUBLIC UI acquired", server.url)
            val bad = entry(session, group, "PUBLIC UI refused", "PUBLIC malformed URL")
            val missing = entry(session, group, "PUBLIC UI no URL", null)
            val source = File(manager.catalog().single().path).readBytes()
            Actor(session, group).use { actor ->
                compose.setContent { Content(actor.state) }
                val selected = session.overview("").groups.single { it.id == group }
                compose.runOnIdle { actor.state.open(selected) }
                compose.onNodeWithTag("bulk-favicon-recursive").assertIsOff().performClick().assertIsOn().performClick().assertIsOff()
                compose.onNodeWithTag("bulk-favicon-replace").assertIsOff()
                assertEquals(0, server.requests.get())
                assertArrayEquals(source, File(manager.catalog().single().path).readBytes())
                compose.onNodeWithTag("load-group-favicons").assertIsEnabled().performClick()
                await { !actor.state.busy && actor.state.rows.size == 3 }
                compose.runOnIdle {
                    assertNull(actor.state.failure)
                    assertEquals(GroupFaviconState.APPLIED, actor.state.rows.single { it.entry == good }.state)
                    assertEquals(GroupFaviconState.FAILED, actor.state.rows.single { it.entry == bad }.state)
                    assertEquals(GroupFaviconState.MISSING_URL, actor.state.rows.single { it.entry == missing }.state)
                }
                compose.onNodeWithTag("bulk-favicon-status-$good").performScrollTo().assertIsDisplayed()
                compose.onNodeWithTag("bulk-favicon-status-$bad").performScrollTo().assertIsDisplayed()
                compose.onNodeWithTag("bulk-favicon-status-$missing").performScrollTo().assertIsDisplayed()
                compose.onNodeWithTag("bulk-favicon-recursive").performScrollTo().assertIsNotEnabled().assertIsOff()
                compose.onNodeWithTag("bulk-favicon-replace").assertIsNotEnabled().assertIsOff()
                assertEquals(2, server.requests.get())
                assertEquals(listOf("/PUBLIC-page", "/PUBLIC.svg"), server.requestPaths())
                assertEquals(2, session.history(FormKind.ENTRY, good).size)
                val confirmed = File(manager.catalog().single().path).readBytes()
                compose.onNodeWithTag("load-group-favicons").assertIsEnabled().performClick()
                await { !actor.state.busy && actor.state.rows.single { it.entry == bad }.state == GroupFaviconState.FAILED }
                assertEquals(2, server.requests.get())
                assertEquals(listOf("/PUBLIC-page", "/PUBLIC.svg"), server.requestPaths())
                assertEquals(2, session.history(FormKind.ENTRY, good).size)
                assertArrayEquals(confirmed, File(manager.catalog().single().path).readBytes())
                compose.onNodeWithTag("cancel-form").performClick()
                compose.onNodeWithTag("bulk-favicon-screen").assertDoesNotExist()
                compose.runOnIdle { assertFalse(actor.state.showing); assertTrue(actor.state.rows.isEmpty()) }
            }
        }
    }

    @Test fun cancelDuringActualHttpAndLockDiscardLateRowsWithoutPublishingOrRefreshingUi() = fixture { session, manager ->
        val group = publicGroup(session)
        PublicFaviconServer(image, holdImage = true).use { server ->
            val target = entry(session, group, "PUBLIC UI cancelled", server.url)
            Actor(session, group).use { actor ->
                compose.setContent { Content(actor.state) }
                val selected = session.overview("").groups.single { it.id == group }
                compose.runOnIdle { actor.state.open(selected) }
                compose.onNodeWithTag("load-group-favicons").performClick()
                await { server.entered.count == 0L }
                assertEquals(listOf("/PUBLIC-page", "/PUBLIC.svg"), server.requestPaths())
                val path = manager.catalog().single().path
                val prepared = File(path).readBytes()
                compose.onNodeWithTag("bulk-favicon-busy").assertIsDisplayed()
                compose.onNodeWithTag("bulk-favicon-recursive").assertIsNotEnabled()
                compose.onNodeWithTag("cancel-form").assertIsEnabled().performClick()
                compose.onNodeWithTag("bulk-favicon-closed").assertExists()
                session.lock()
                server.release.countDown()
                await { actor.idle }
                compose.runOnIdle {
                    assertFalse(actor.state.showing)
                    assertFalse(actor.state.busy)
                    assertFalse(actor.state.started)
                    assertTrue(actor.state.rows.isEmpty())
                    assertNull(actor.state.failure)
                }
                assertEquals(0, actor.refreshes.get())
                assertArrayEquals(prepared, File(path).readBytes())
                manager.close(session)
                val reopened = manager.open(path, password)
                assertEquals(IconView.Default, reopened.entry(target).icon)
                assertEquals(1, reopened.history(FormKind.ENTRY, target).size)
                compose.onNodeWithTag("bulk-favicon-screen").assertDoesNotExist()
            }
        }
    }

    @Composable private fun Content(state: GroupFaviconsState) {
        TaypeerTheme(ThemeMode.SYSTEM, UiLanguage.ENGLISH) {
            if (state.showing) GroupFaviconsScreen(state) else Box(Modifier.testTag("bulk-favicon-closed"))
        }
    }
    private fun await(condition: () -> Boolean) = compose.waitUntil(20_000, condition)

    private inner class Actor(session: DocumentSession, group: String) : AutoCloseable {
        private val job = SupervisorJob()
        private val scope = CoroutineScope(job + Dispatchers.Main.immediate)
        private val binding = session.overview("").let { GroupFaviconBinding(session, it.database, it.generation, 1) }
        val refreshes = AtomicInteger()
        val state = GroupFaviconsState(scope, { binding }, { it === binding }, { refreshes.incrementAndGet() })
        val idle get() = job.children.none { it.isActive }
        init { assertTrue(session.overview("").groups.any { it.id == group }) }
        override fun close() {
            compose.runOnIdle { state.close() }
            job.cancel()
            runBlocking { job.join() }
        }
    }
    private inline fun fixture(block: (DocumentSession, DocumentManager) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-group-favicons-ui-${UUID.randomUUID()}")
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                DocumentManager(context, host).use { manager ->
                    block(manager.create("PUBLIC group favicon UI database", null, newOperation(), password), manager)
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }
    private fun entry(session: DocumentSession, group: String, title: String, url: String?): String {
        session.beginCreateEntry(group)
        session.patchEntry(EntryTextField.TITLE, TextEdit.Set(title))
        session.patchEntry(EntryTextField.URL, url?.let(TextEdit::Set) ?: TextEdit.Clear)
        val ready = session.editor()
        assertEquals(SaveState.SAVED, session.save(ready.form.draft, ready.form.revision, newOperation()).state)
        session.deleteForm(ready.form.draft)
        return ready.form.target!!
    }
}
