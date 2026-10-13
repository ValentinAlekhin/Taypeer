package dev.taypeer

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*
import java.io.File

/** Saved FIG form + real navigation, encrypted publication, process closure and adopted catalog. */
class RecoveryUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var document: PublicDocumentUiFixture
    private val state get() = document.state
    private val password = "PUBLIC recovered UI master password"
    private val name = "  PUBLIC recovered UI exact name  "
    private var sourcePath = ""
    private var sourceRoot = ""
    private var entry = ""
    private var group = ""

    @Before fun createPublicSource() {
        document = PublicDocumentUiFixture(compose)
        document.create()
        group = document.group("PUBLIC recovery UI group")
        entry = document.entry(group, PublicEntryFields(title = "PUBLIC original UI entry"))
        compose.runOnIdle {
            val source = state.catalog.single { it.database == state.overview?.database }
            sourcePath = source.path; sourceRoot = source.root
        }
    }
    @After fun restoreOnlyFixturePreferences() {
        if (::document.isInitialized) {
            compose.runOnIdle { state.lock() }
            document.restorePreferences()
        }
    }
    private fun fields() {
        compose.onNodeWithTag("recovery-name").performTextReplacement(name)
        compose.onNodeWithTag("recovery-password").performTextReplacement(password)
        compose.onNodeWithTag("recovery-repeat").performTextReplacement(password)
    }
    private fun awaitAdopted() {
        document.await { state.overview?.name == name && !state.opening && !state.navigating && !state.recovery.showing &&
            state.catalog.any { it.database == state.overview?.database && it.root != sourceRoot && it.path != sourcePath } }
        compose.runOnIdle {
            val result = state.catalog.single { it.database == state.overview?.database }
            assertNotEquals(sourceRoot, result.root)
            assertNotEquals(sourcePath, result.path)
            assertTrue(state.overview!!.managing)
            assertTrue(state.recovery.pending.none { it.source.path == sourcePath })
        }
    }

    @Test fun stayAndDiscardGuardRecoveryWithoutSavingOrChangingTheSource() {
        val before = File(sourcePath).readBytes()
        compose.onNodeWithTag("edit-entry").performClick()
        document.waitFor("entry-title")
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC unsaved discarded recovery title")
        document.menu("recover-management")
        document.waitFor("navigation-stay")
        compose.onNodeWithTag("navigation-stay").performClick()
        document.waitAbsent("navigation-stay")
        compose.onNodeWithTag("entry-title").assertTextEquals("PUBLIC unsaved discarded recovery title")
        compose.runOnIdle { assertFalse(state.recovery.showing); assertNull(state.recovery.target) }
        assertArrayEquals(before, File(sourcePath).readBytes())
        document.menu("recover-management")
        document.waitFor("navigation-discard")
        compose.onNodeWithTag("navigation-discard").performClick()
        document.waitFor("recovery-screen")
        fields()
        compose.onNodeWithTag("confirm-recovery").performClick()
        awaitAdopted()
        assertArrayEquals(before, File(sourcePath).readBytes())
        document.inspect(group, entry)
        compose.onNode(hasText("PUBLIC original UI entry") and
            hasAnyAncestor(hasTestTag("route-inspected"))).assertExists()
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { state.entryHistory.size == 1 }
    }

    @Test fun cancelAndTypedFieldRejectionRetainInputUntilOneExplicitSuccessfulCreate() {
        val before = File(sourcePath).readBytes()
        document.menu("recover-management")
        document.waitFor("recovery-screen")
        fields()
        compose.onNodeWithTag("cancel-form").performClick()
        document.waitAbsent("recovery-screen")
        compose.runOnIdle { assertFalse(state.recovery.showing); assertTrue(state.recovery.password.isEmpty()); assertNull(state.recovery.target) }
        assertArrayEquals(before, File(sourcePath).readBytes())
        document.menu("recover-management")
        document.waitFor("recovery-screen")
        compose.onNodeWithTag("recovery-password").performTextReplacement(password)
        compose.onNodeWithTag("recovery-repeat").performTextReplacement(password)
        compose.onNodeWithTag("confirm-recovery").performClick()
        document.await { state.recovery.failure == RecoveryFailure.INPUT && !state.recovery.busy }
        compose.onNodeWithTag("recovery-error").assertExists()
        compose.runOnIdle { assertEquals(password, state.recovery.password); assertEquals(password, state.recovery.repeated); assertFalse(state.recovery.frozen) }
        assertArrayEquals(before, File(sourcePath).readBytes())
        compose.onNodeWithTag("recovery-name").performTextReplacement(name)
        compose.onNodeWithTag("confirm-recovery").performClick()
        awaitAdopted()
        assertArrayEquals(before, File(sourcePath).readBytes())
        document.inspect(group, entry)
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { state.entryHistory.size == 1 }
    }

    @Test fun explicitSaveBeforeRecoveryPreservesTheSavedRevisionAndDoesNotSaveAgain() {
        val beforeEdit = File(sourcePath).readBytes()
        compose.onNodeWithTag("edit-entry").performClick()
        document.waitFor("entry-title")
        compose.onNodeWithTag("entry-title").performTextReplacement("PUBLIC explicitly saved recovery title")
        document.menu("recover-management")
        document.waitFor("navigation-save")
        compose.onNodeWithTag("navigation-save").performClick()
        document.waitFor("recovery-screen")
        val saved = File(sourcePath).readBytes()
        assertFalse(beforeEdit.contentEquals(saved))
        fields()
        compose.onNodeWithTag("confirm-recovery").performClick()
        awaitAdopted()
        assertArrayEquals(saved, File(sourcePath).readBytes())
        document.inspect(group, entry)
        compose.onNode(hasText("PUBLIC explicitly saved recovery title") and
            hasAnyAncestor(hasTestTag("route-inspected"))).assertExists()
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        document.await { state.entryHistory.size == 2 }
    }
}
