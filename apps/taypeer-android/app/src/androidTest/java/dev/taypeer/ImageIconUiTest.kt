package dev.taypeer

import android.graphics.Color
import androidx.activity.compose.setContent
import androidx.lifecycle.ViewModelStore
import java.util.concurrent.atomic.AtomicInteger
import android.net.Uri
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import dev.taypeer.bridge.*
import org.junit.After
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.UUID

/** PUBLIC UI actions stage originals through real Rust workers; only Save changes history. */
class ImageIconUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val fixture by lazy { PublicDocumentUiFixture(compose) }
    private val state get() = fixture.state
    private val svg = """<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect width="32" height="16" fill="#14dc28"/></svg>""".toByteArray()

    @After fun restorePreferences() = fixture.restorePreferences()

    private fun greenPreview(parentTag: String? = null) {
        val image = hasTestTag("rendered-icon").let { matcher ->
            if (parentTag == null) matcher else matcher and hasAnyAncestor(hasTestTag(parentTag))
        }
        // Clickable library rows merge the image's semantics into their own tagged node.
        fixture.await { compose.onAllNodes(image, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        val bitmap = compose.onNode(image, useUnmergedTree = true).assertIsDisplayed().captureToImage().asAndroidBitmap()
        try { assertEquals(Color.rgb(20, 220, 40), bitmap.getPixel(bitmap.width / 2, bitmap.height / 2)) }
        finally { bitmap.recycle() }
    }

    private fun appearance() {
        compose.onNodeWithTag("entry-tab-2").performScrollTo().performClick()
        fixture.waitFor("icon-picker")
    }

    private fun chooseLucide(key: String) {
        compose.onNodeWithTag("icon-picker").performScrollTo().performClick()
        compose.onNodeWithTag("icon-source-lucide").performClick()
        compose.onNodeWithTag("lucide-search").performTextReplacement(key)
        compose.onNodeWithTag("icon-key-$key").performClick()
        compose.onNodeWithTag("choose-lucide").performClick()
        fixture.await { state.appearance?.lucide == key && state.activeForm?.dirty == true }
    }

    @Test fun filePreviewStagesOneFormAndSavedVersionKeepsOriginalAfterCurrentLucideChange() {
        fixture.create()
        val group = fixture.group("PUBLIC icon UI group")
        val entry = fixture.entry(group, PublicEntryFields(title = "PUBLIC icon UI entry"))
        compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
        fixture.await { state.entryHistory.size == 1 }
        var originalVersion = ""
        compose.runOnIdle { originalVersion = state.entryHistory.single().id }
        compose.onNodeWithTag("edit-entry").performClick()
        appearance()
        val before = fixture.savedBytes()
        val selected = File(compose.activity.cacheDir, "PUBLIC-icon-${UUID.randomUUID()}.svg").apply { writeBytes(svg) }
        try {
            compose.runOnIdle { state.prepareIconFile { operation -> state.selectedIconFile(operation, Uri.fromFile(selected)) } }
            fixture.waitFor("accept-selected-icon")
            compose.onNodeWithTag("rendered-icon").assertDoesNotExist()
            assertArrayEquals(before, fixture.savedBytes())
            compose.onNodeWithTag("accept-selected-icon").performScrollTo().performClick()
            fixture.await { state.icons.attempt(state.iconTarget()) == null && state.appearance?.image != null }
            greenPreview()
            assertArrayEquals(before, fixture.savedBytes())
            compose.onNodeWithTag("save-form").performClick()
            fixture.await { state.activeForm == null && state.inspectedEntry?.id == entry && !state.navigating }
            assertEquals(IconView.Image, state.inspectedEntry?.icon)
            greenPreview()
            assertFalse(before.contentEquals(fixture.savedBytes()))
            compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
            fixture.await { state.entryHistory.size == 2 }
            var imageVersion: HistoryRow? = null
            compose.runOnIdle { imageVersion = state.entryHistory.single { it.id != originalVersion } }

            compose.onNodeWithTag("edit-entry").performClick()
            appearance()
            chooseLucide("server")
            compose.onNodeWithTag("rendered-icon").assertDoesNotExist()
            compose.onNodeWithTag("save-form").performClick()
            fixture.await { state.activeForm == null && state.inspectedEntry?.icon == IconView.Lucide("server") }
            compose.runOnIdle { state.showHistory(FormKind.ENTRY, entry) }
            fixture.waitFor("history-list-screen")
            compose.onNodeWithTag("history-version-${imageVersion!!.id}").performScrollTo().performClick()
            fixture.waitFor("saved-version-screen")
            compose.onNodeWithTag("entry-tab-2").performScrollTo().performClick()
            fixture.await { state.historicalProperties?.appearance?.image != null }
            greenPreview()
            val saved = fixture.savedBytes()
            compose.onNodeWithTag("icon-picker").assertDoesNotExist()
            compose.runOnIdle { state.lock() }
            fixture.waitFor("master-password")
            compose.runOnIdle { assertNull(state.renderedIcon(IconScope.Revision(entry, imageVersion!!.id))) }
            compose.onNodeWithTag("rendered-icon").assertDoesNotExist()
            assertArrayEquals(saved, fixture.savedBytes())
        } finally { assertTrue(selected.delete()) }
    }

    @Test fun openingSourcesAndTypingNeverFetchButExplicitDownloadShowsLocalPreview() {
        fixture.create()
        val group = fixture.group("PUBLIC network icon UI group")
        val entry = fixture.entry(group, PublicEntryFields(title = "PUBLIC explicit URL icon"))
        compose.onNodeWithTag("edit-entry").performClick()
        appearance()
        val before = fixture.savedBytes()
        PublicIconServer(svg).use { server ->
            compose.onNodeWithTag("icon-picker").performScrollTo().performClick()
            compose.onNodeWithTag("icon-source-url").performClick()
            compose.onNodeWithTag("icon-image-url").performTextReplacement(server.url)
            compose.runOnIdle { assertEquals(0, server.requests.get()) }
            assertArrayEquals(before, fixture.savedBytes())
            compose.onNodeWithTag("download-icon").performClick()
            fixture.await { state.icons.attempt(state.iconTarget()) == null && state.appearance?.image != null }
            fixture.waitAbsent("icon-image-url")
            greenPreview()
            assertEquals(1, server.requests.get())
            assertArrayEquals(before, fixture.savedBytes())
            server.close()
            compose.onNodeWithTag("save-form").performClick()
            fixture.await { state.activeForm == null && state.inspectedEntry?.id == entry }
            greenPreview()
            compose.onNodeWithTag("entry-tab-4").performScrollTo().performClick()
            fixture.await { state.entryHistory.size == 2 }
            assertEquals(1, server.requests.get())
        }
    }

    @Test fun selectedGroupFileSurvivesBackgroundAndCannotApplyToAnotherDatabase() {
        fixture.create()
        compose.runOnIdle { state.beginMetadata(FormKind.NEW_GROUP, parent = null) }
        fixture.waitFor("metadata-name")
        compose.onNodeWithTag("metadata-name").performTextReplacement("PUBLIC background image group")
        val before = fixture.savedBytes()
        var copy: WorkingCopyView? = null
        var operation = ""
        var draft = ""
        compose.runOnIdle {
            copy = state.catalog.single { it.database == state.overview?.database }
            draft = state.activeForm!!.draft
            state.prepareIconFile { operation = it }
        }
        fixture.await { operation.isNotEmpty() && !state.navigating }
        val selected = File(compose.activity.cacheDir, "PUBLIC-group-icon-${UUID.randomUUID()}.svg").apply { writeBytes(svg) }
        try {
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            fixture.await { state.overview == null && state.activeForm == null }
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            fixture.waitFor("choose-another-database")
            compose.onNodeWithTag("choose-another-database").performClick()
            compose.runOnIdle { state.create("PUBLIC unrelated icon database", null, "PUBLIC unrelated icon master", newOperation()) }
            fixture.await { state.overview != null && state.overview?.database != copy!!.database && !state.navigating }
            compose.runOnIdle { state.selectedIconFile(operation, Uri.fromFile(selected)) }
            fixture.waitFor("accept-selected-icon")
            compose.onNodeWithTag("accept-selected-icon").assertIsNotEnabled()
            assertTrue(state.overview!!.groups.isEmpty())
            assertArrayEquals(before, fixture.savedBytes())

            compose.runOnIdle { state.unlock(copy!!, "PUBLIC lifecycle UI master password") }
            fixture.await { state.overview?.database == copy!!.database && state.activeForm?.draft == draft && !state.navigating }
            compose.onNodeWithTag("accept-selected-icon").assertIsEnabled().performScrollTo().performClick()
            fixture.await { state.metadata?.source?.icon == IconView.Image && !state.editingBlocked }
            greenPreview()
            assertArrayEquals(before, fixture.savedBytes())
            compose.onNodeWithTag("save-form").performClick()
            fixture.await { state.activeForm == null && state.overview?.groups?.singleOrNull()?.icon == IconView.Image }
            var groupId = ""
            compose.runOnIdle { groupId = state.overview!!.groups.single().id }
            compose.onNodeWithTag("group-menu").performClick()
            fixture.waitFor("database-drawer-open")
            greenPreview("select-group-$groupId")
            assertEquals("PUBLIC background image group", state.overview!!.groups.single().name)
        } finally { assertTrue(selected.delete()) }
    }

    @Test fun preparedLostReplyRetriesNativeReceiptBeforePinningAndNeverOpensMissingProvider() {
        val application = compose.activity.application as TaypeerApplication
        val real = NativeIconStageExecutor(application)
        val pins = AtomicInteger()
        val executions = mutableListOf<IconRequest>()
        val executor = object : IconStageExecutor {
            override fun pin(session: DocumentSession): LocalForm { pins.incrementAndGet(); return real.pin(session) }
            override fun execute(session: DocumentSession, request: IconRequest): LocalForm {
                val value = real.execute(session, request)
                synchronized(executions) { executions.add(request) }
                if (executions.size == 1) throw AndroidException.CommitUncertain()
                return value
            }
        }
        val store = ViewModelStore()
        lateinit var local: DocumentState
        compose.runOnIdle { local = DocumentState(application, executor); store.put("PUBLIC icon retry state", local) }
        val selected = File(application.cacheDir, "PUBLIC-retry-icon-${UUID.randomUUID()}.svg").apply { writeBytes(svg) }
        try {
            compose.runOnIdle { local.create("PUBLIC prepared icon retry", null, "PUBLIC prepared retry master", newOperation()) }
            fixture.await { local.overview != null && !local.navigating && local.catalog.any { it.database == local.overview?.database } }
            compose.runOnIdle { local.beginMetadata(FormKind.NEW_GROUP, parent = null) }
            fixture.await { local.metadata != null && !local.navigating }
            compose.runOnIdle { local.editMetadata(name = "PUBLIC exact retry group") }
            var target: IconTarget? = null
            var path = ""
            compose.runOnIdle {
                target = local.iconTarget()
                path = local.catalog.single { it.database == local.overview?.database }.path
                local.prepareIconFile { operation -> local.selectedIconFile(operation, Uri.fromFile(selected)) }
            }
            fixture.await { local.icons.file?.uri != null && !local.navigating }
            val before = File(path).readBytes()
            compose.runOnIdle { local.acceptIconFile() }
            fixture.await { local.icons.attempt(target)?.uncertain == true }
            var request: IconRequest? = null
            var firstToken = 0L
            compose.runOnIdle {
                val attempt = local.icons.attempt(target)!!
                assertTrue(attempt.prepared); assertTrue(local.editingBlocked)
                request = attempt.request; firstToken = attempt.token
                assertNull(local.icons.begin(attempt.request.copy(operation = newOperation())))
            }
            assertArrayEquals(before, File(path).readBytes())
            assertTrue(selected.delete())
            val preparedPins = pins.get()
            compose.runOnIdle { local.retryIcon() }
            fixture.await { local.icons.attempt(target) == null && local.metadata?.source?.icon == IconView.Image }
            assertEquals(preparedPins, pins.get())
            assertEquals(listOf(request, request), synchronized(executions) { executions.toList() })
            compose.runOnIdle {
                assertFalse(local.icons.failed(request!!, firstToken, AndroidException.Icon(IconFailure.FILE)))
                assertFalse(local.editingBlocked)
            }
            assertArrayEquals(before, File(path).readBytes())
            compose.runOnIdle { local.saveNow() }
            fixture.await { local.activeForm == null && local.overview?.groups?.size == 1 && !local.navigating }
            var group = ""
            compose.runOnIdle { group = local.overview!!.groups.single().id; local.showHistory(FormKind.GROUP, group) }
            fixture.await { local.history?.target == group }
            assertEquals(1, local.history!!.rows.size)
        } finally { selected.delete(); compose.runOnIdle { store.clear() } }
    }

    @Test fun replacingUnacceptedFileKeepsNewPinForCleanFormAcrossBackground() {
        fixture.create()
        val group = fixture.group("PUBLIC repeated picker group")
        val entry = fixture.entry(group, PublicEntryFields(title = "PUBLIC clean pinned entry"))
        compose.onNodeWithTag("edit-entry").performClick()
        appearance()
        val before = fixture.savedBytes()
        var draft = ""
        val operations = mutableListOf<String>()
        compose.runOnIdle {
            draft = state.activeForm!!.draft
            assertFalse(state.activeForm!!.dirty)
            state.prepareIconFile { operations.add(it) }
        }
        fixture.await { operations.size == 1 && !state.navigating }
        compose.runOnIdle { state.prepareIconFile { operations.add(it) } }
        fixture.await { operations.size == 2 && !state.navigating }
        assertNotEquals(operations[0], operations[1])
        val selected = File(compose.activity.cacheDir, "PUBLIC-second-icon-${UUID.randomUUID()}.svg").apply { writeBytes(svg) }
        try {
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            fixture.await { state.overview == null }
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            fixture.waitFor("master-password")
            compose.runOnIdle {
                state.selectedIconFile(operations[0], Uri.fromFile(selected))
                assertNull(state.icons.file?.uri)
                state.selectedIconFile(operations[1], Uri.fromFile(selected))
            }
            compose.onNodeWithTag("master-password").performTextReplacement("PUBLIC lifecycle UI master password")
            compose.onNodeWithTag("authenticate").performClick()
            fixture.await { state.activeForm?.draft == draft && state.entry?.source?.entry == entry && !state.navigating }
            compose.onNodeWithTag("accept-selected-icon").assertIsEnabled()
            assertFalse(state.activeForm!!.dirty)
            assertArrayEquals(before, fixture.savedBytes())
            compose.onNodeWithTag("accept-selected-icon").performScrollTo().performClick()
            fixture.await { state.appearance?.image != null && !state.editingBlocked }
            greenPreview()
            assertArrayEquals(before, fixture.savedBytes())
        } finally { assertTrue(selected.delete()) }
    }

    @Test fun eagerListLoadsOnlyVisibleNativeImagesAndRestoresThemWhenScrollingBack() {
        val application = compose.activity.application as TaypeerApplication
        val store = ViewModelStore()
        lateinit var local: DocumentState
        compose.runOnIdle { local = DocumentState(application); store.put("PUBLIC icon visibility state", local) }
        // The hash-bound corpus was generated by this real Native scenario's
        // original image imports and 69 clones; its source is preserved with it.
        val selected = File(application.cacheDir, "PUBLIC-list-corpus-${UUID.randomUUID()}.taypeer")
        val corpus = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().context.assets
            .open("seventy-image-entries.taypeer").use { it.readBytes() }
        assertEquals(4_567_715, corpus.size)
        val checksum = java.security.MessageDigest.getInstance("SHA-256").digest(corpus)
            .joinToString("") { "%02x".format(it.toInt() and 255) }
        assertEquals("a965939a4ad83a973e93655d5b9ab435fd50c37c1acf08bfa6a4073985a3ea6d", checksum)
        selected.writeBytes(corpus)
        try {
            compose.runOnIdle { local.import(Uri.fromFile(selected), "PUBLIC image list master") }
            // Corpus import authenticates the full archive/history and publishes
            // its working copy. Preview responsiveness is checked separately below.
            try {
                compose.waitUntil(120_000) { local.overview?.entries?.size == 70 && !local.opening && !local.navigating &&
                    local.catalog.any { it.database == local.overview?.database } }
            } catch (timeout: ComposeTimeoutException) {
                var flags = ""
                compose.runOnIdle { flags = "entries=${local.overview?.entries?.size}, opening=${local.opening}, navigating=${local.navigating}, failure=${local.error}, catalog=${local.catalog.size}" }
                throw AssertionError("PUBLIC corpus import did not finish; $flags", timeout)
            }
            val entries = local.overview!!.entries.sortedBy { it.title }.map { it.id }
            assertEquals(70, entries.toSet().size)
            // Its generator's dedicated profile can already have valid admission.
            // Exercise previews with the actual Native permission; rendering must
            // not depend on whether this PUBLIC archive is known to that profile.
            val bytes = File(local.catalog.single { it.database == local.overview?.database }.path).readBytes()
            assertArrayEquals(corpus, bytes)
            compose.activityRule.scenario.onActivity { activity -> activity.setContent {
                TaypeerTheme { LibraryScreen(local, rememberAttachmentActions(local), copy = {}) }
            } }
            fun loaded(id: String) {
                try { fixture.await { local.renderedIcon(IconScope.Entry(id)) != null } }
                catch (timeout: ComposeTimeoutException) {
                    var phase = ""
                    compose.runOnIdle { phase = local.iconPreviewPhase(IconScope.Entry(id)) + "/" +
                        local.readPreviewDiagnostic { it.iconPreviewStage(IconScope.Entry(id)).name } }
                    throw AssertionError("Actual visible PUBLIC image preview did not load; phase=$phase", timeout)
                }
            }
            fun green(id: String) {
                val image = compose.onNode(
                    hasTestTag("rendered-icon") and hasAnyAncestor(hasTestTag("entry-$id")),
                    useUnmergedTree = true,
                ).assertIsDisplayed().captureToImage().asAndroidBitmap()
                try { assertEquals(Color.rgb(20, 220, 40), image.getPixel(image.width / 2, image.height / 2)) }
                finally { image.recycle() }
            }
            // Other real PUBLIC scenarios may leave pending-operation banners in
            // the same profile. Bring this row into the viewport before testing
            // visibility-gated loading; an offscreen icon must remain unloaded.
            compose.runOnIdle { local.activity() }
            compose.onNodeWithTag("entry-${entries.first()}").performScrollTo().assertIsDisplayed()
            loaded(entries.first()); green(entries.first())
            compose.runOnIdle { assertNull(local.renderedIcon(IconScope.Entry(entries[35]))) }
            // Visit all actual rows to exercise eviction, not only two endpoints.
            entries.forEach { id ->
                // Compose semantic scrolling bypasses Activity.onUserInteraction.
                // Model the user's scroll input here; Native completion never refreshes idle.
                compose.runOnIdle { local.activity() }
                compose.onNodeWithTag("entry-$id").performScrollTo()
                loaded(id)
                compose.runOnIdle {
                    assertTrue("Actual rendered previews must remain bounded",
                        entries.count { local.renderedIcon(IconScope.Entry(it)) != null } <= 64)
                }
            }
            green(entries.last())
            var evicted = ""
            compose.runOnIdle {
                // Initial visible requests finish on independent IO dispatchers;
                // their completion order need not equal the alphabetical row order.
                evicted = entries.firstOrNull { local.renderedIcon(IconScope.Entry(it)) == null }
                    ?: throw AssertionError("Seventy visited rows must evict an offscreen preview from a 64-item cache")
            }
            compose.runOnIdle { local.activity() }
            compose.onNodeWithTag("entry-$evicted").performScrollTo()
            loaded(evicted); green(evicted)
            compose.runOnIdle { local.activity() }
            compose.onNodeWithTag("entry-${entries.first()}").performScrollTo()
            loaded(entries.first()); green(entries.first())
            assertArrayEquals(bytes, File(local.catalog.single { it.database == local.overview?.database }.path).readBytes())
        } finally { assertTrue(selected.delete()); compose.runOnIdle { store.clear() } }
    }

}
