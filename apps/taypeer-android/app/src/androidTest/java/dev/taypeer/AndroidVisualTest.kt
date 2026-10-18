package dev.taypeer

import android.graphics.Bitmap
import android.view.WindowInsets
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*
import java.io.File
import java.util.concurrent.atomic.AtomicReference

/** Captures retained product builders with public fixtures, independent of an open database. */
class AndroidVisualTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private data class Viewport(val size: IntSize, val position: Offset, val clipped: Rect)
    private val viewport = AtomicReference<Viewport?>()

    @Test fun captureProductSurfaces() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val arguments = InstrumentationRegistry.getArguments()
        val theme = if (arguments.getString("qaTheme") == "light") ThemeMode.LIGHT else ThemeMode.DARK
        val language = if (arguments.getString("qaLocale") == "ru") UiLanguage.RUSSIAN else UiLanguage.ENGLISH
        val scale = arguments.getString("qaFontScale")?.toFloatOrNull() ?: 1f
        val width = arguments.getString("qaWidthDp")?.toIntOrNull() ?: 390
        val height = arguments.getString("qaHeightDp")?.toIntOrNull() ?: 788
        val cases = arguments.getString("qaScreen")?.split(',') ?: visualCases
        var current by mutableStateOf(cases.first())
        val folder = File(instrumentation.targetContext.filesDir, "qa-screenshots").apply { mkdirs() }
        // Only this QA Activity owns the viewport. Product secure-window policy is untouched.
        compose.activityRule.scenario.onActivity { activity ->
            activity.enableEdgeToEdge()
            activity.window.attributes = activity.window.attributes.apply {
                layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            }
            WindowCompat.getInsetsController(activity.window, activity.window.decorView)
                .hide(WindowInsetsCompat.Type.systemBars())
        }
        compose.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, scale)) {
                TaypeerTheme(theme, language) {
                    Box(Modifier.fillMaxSize()) {
                        // Respect the drawable constraints, then fail if the requested viewport does not fit.
                        // requiredSize would center an oversized child and could capture clipped system pixels.
                        Surface(Modifier.size(width.dp, height.dp).align(Alignment.TopStart).testTag("visual-root")
                            .onGloballyPositioned { coordinates ->
                                viewport.set(Viewport(coordinates.size, coordinates.positionInWindow(), coordinates.boundsInWindow()))
                            }) {
                            key(current) { VisualSurface(current, theme, language) }
                        }
                    }
                }
            }
        }
        compose.waitUntil(5_000) {
            compose.activity.window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.systemBars()) == false
        }
        cases.forEach { case ->
            require(case in visualCases)
            compose.runOnIdle { current = case }
            if (case == "generator" || case == "generator-use" || case == "passphrase") compose.waitUntil(20_000) {
                compose.onAllNodes(hasTestTag("generated-password")).fetchSemanticsNodes().isNotEmpty()
            }
            if (case == "trash-selected") compose.onNodeWithTag("trash-PUBLIC-old-server").performClick()
            if (case == "settings-database") {
                compose.onNodeWithTag("settings-scope").performClick()
                compose.onNodeWithTag("settings-scope-database").performClick()
            }
            if (case == "change-password" || case == "revoke") {
                compose.onNodeWithTag("rotation-new-password").performTextReplacement("PUBLIC visual master password")
                compose.onNodeWithTag("rotation-repeat-password").performTextReplacement("PUBLIC visual master password")
                // The real form action clears focus/IME; this public fixture has a no-op callback.
                compose.onNodeWithTag("apply-password-change").performClick()
            }
            compose.runOnIdle {
                compose.activity.currentFocus?.clearFocus()
                compose.activity.getSystemService(InputMethodManager::class.java)
                    .hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0)
            }
            compose.waitUntil(5_000) {
                compose.activity.window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.ime()) == false
            }
            compose.waitForIdle()
            if (case == "search-scope") {
                // A composed modal can still be outside its visible position during its entrance.
                // Capture only the actual open sheet, rather than accepting its background screen.
                val visibleTags = listOf("search-scope-sheet", "search-current", "search-all-unlocked")
                compose.waitUntil(5_000) {
                    visibleTags.all { tag ->
                        try { compose.onNodeWithTag(tag).assertIsDisplayed(); true }
                        catch (_: AssertionError) { false }
                    }
                }
                visibleTags.forEach { compose.onNodeWithTag(it).assertIsDisplayed() }
            }
            assertViewport(width, height)
            when (case) {
                "entries" -> {
                    compose.onNodeWithTag("group-menu").assertIsDisplayed()
                    compose.onNodeWithTag("screen-actions").assertIsDisplayed()
                    compose.onNodeWithTag("add-entry").assertIsDisplayed()
                }
                "protection", "limits" -> {
                    compose.onNodeWithTag("navigation-back").assertIsDisplayed()
                    compose.onNodeWithTag("cancel-form").assertIsDisplayed()
                    compose.onNodeWithTag("apply-policy").assertIsDisplayed()
                }
                "settings-database" -> compose.onNodeWithTag("settings-limits").assertIsDisplayed()
                "group-restore-version", "database-restore-version" -> {
                    compose.onNodeWithTag("navigation-back").assertIsDisplayed()
                    compose.onNodeWithTag("confirm-metadata-restore").assertIsDisplayed()
                }
            }
            val bitmap = compose.onNodeWithTag("visual-root").captureToImage().asAndroidBitmap()
            File(folder, "$case.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }

    private fun assertViewport(width: Int, height: Int) {
        compose.runOnIdle {
            val actual = checkNotNull(viewport.get()) { "PUBLIC visual root has not been placed" }
            assertEquals("Requested visual width must fit actual drawable constraints", width, actual.size.width)
            assertEquals("Requested visual height must fit actual drawable constraints", height, actual.size.height)
            assertEquals("Visual left must not be clipped", actual.position.x, actual.clipped.left, 0.5f)
            assertEquals("Visual top must not be clipped", actual.position.y, actual.clipped.top, 0.5f)
            assertEquals("Visual right must not be clipped", actual.position.x + width, actual.clipped.right, 0.5f)
            assertEquals("Visual bottom must not be clipped", actual.position.y + height, actual.clipped.bottom, 0.5f)
            val decor = compose.activity.window.decorView
            val origin = IntArray(2)
            decor.getLocationInWindow(origin)
            assertTrue("Full visual root must stay inside Activity drawable bounds",
                actual.position.x >= origin[0] && actual.position.y >= origin[1] &&
                    actual.position.x + width <= origin[0] + decor.width &&
                    actual.position.y + height <= origin[1] + decor.height)
        }
    }
}

private val visualCases = listOf("welcome", "entries", "drawer", "entry", "edit", "entry-appearance", "edit-appearance",
    "entry-properties", "entry-history", "entry-additional", "edit-additional", "generator", "generator-use", "passphrase", "settings", "settings-database", "receive", "invitation", "trash", "trash-selected", "clone", "move", "relay", "relay-custom", "drafts",
    "history-list", "history-version", "history-version-additional", "history-version-appearance", "history-version-properties", "history-version-history",
    "history-compare", "restore-version", "change-password", "revoke", "icon-picker", "lucide-picker", "image-url", "group-create", "group-edit", "recover-control", "transfer-control", "accept-control", "device-menu",
    "search-current", "search-all", "search-empty", "search-scope", "protection", "limits", "bulk-icons",
    "group-history-version", "group-history-compare", "group-restore-version",
    "database-history-version", "database-history-compare", "database-restore-version")

private val publicGroups = listOf(
    GroupRow("PUBLIC-work", null, "Work", null, IconView.Default),
    GroupRow("PUBLIC-services", "PUBLIC-work", "Services", null, IconView.Default),
    GroupRow("PUBLIC-infrastructure", "PUBLIC-work", "Infrastructure", null, IconView.Lucide("server")),
    GroupRow("PUBLIC-personal", null, "Personal", null, IconView.Default),
    GroupRow("PUBLIC-archive", null, "Archive", null, IconView.Default),
)
private val publicOverview = DocumentOverview("PUBLIC-database", 1uL, "Working", null, true, true, publicGroups,
    listOf("Cloudflare", "Figma", "GitHub", "Linear", "Notion", "Vercel").mapIndexed { index, title ->
        EntryRow("PUBLIC-$index", "PUBLIC-work", title, listOf("admin@studio.example", "demo@studio.example", "demo", "demo", "demo@studio.example", "demo")[index], null, null, null, false, IconView.Default, 1_758_010_860_000)
    })

@Composable
private fun VisualSurface(case: String, theme: ThemeMode, language: UiLanguage) {
    when (case) {
        "group-history-version", "group-history-compare", "group-restore-version",
        "database-history-version", "database-history-compare", "database-restore-version" -> {
            val kind = if (case.startsWith("group")) FormKind.GROUP else FormKind.DATABASE
            val mode = when {
                case.endsWith("compare") -> MetadataHistoryMode.COMPARE
                case.contains("restore") -> MetadataHistoryMode.RESTORE
                else -> MetadataHistoryMode.SAVED
            }
            val value = publicMetadataRevision(kind, language)
            MetadataHistoryContent(kind, mode, value, null, null, value.saved.parent, false,
                false, false, true, {}, {}, {}, {}, {}, {}, {}, {})
        }
        "bulk-icons" -> GroupFaviconsForm(localizedOverview(language).groups.first().name,
            false, false, true, false, false, false, emptyList(), null, {}, {}, {}, {})
        "protection", "limits" -> Scaffold(topBar = {
            PhoneBar(stringResource(if (case == "protection") R.string.protection_title else R.string.limits_title), back = {})
        }, bottomBar = { FormActions({}, {}, enabled = true, tag = "apply-policy", label = R.string.apply) }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp)) {
                if (case == "protection") ProtectionContent(publicPolicy, 1000u, null, false, true,
                    localizedOverview(language).name, true, {}, {})
                else LimitsContent(publicPolicy, "10", "100", true, {}, {})
            }
        }
        "search-scope" -> Box(Modifier.fillMaxSize()) {
            VisualSurface("search-current", theme, language)
            SearchScopeSheet(SearchScope.CURRENT, {}, {})
        }
        "search-current", "search-all", "search-empty" -> Scaffold(contentWindowInsets = WindowInsets(0, 0, 0, 0), topBar = {
            PhoneBar(stringResource(R.string.search), back = {}) {
                IconAction(R.drawable.ic_x, R.string.clear_search) {}
            }
        }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState())) {
                val overview = localizedOverview(language)
                val working = WorkingCopyView(overview.database, "PUBLIC-search-root", "/PUBLIC/search-working")
                val rows = listOf(
                    SearchResultRow(working, overview.generation, overview.name,
                        overview.groups.single { it.id == "PUBLIC-work" }.name,
                        overview.entries.single { it.title == "Figma" }),
                    SearchResultRow(working, overview.generation, overview.name,
                        overview.groups.single { it.id == "PUBLIC-infrastructure" }.name,
                        overview.entries.single { it.title == "Cloudflare" }.copy(group = "PUBLIC-infrastructure")),
                ) + if (case == "search-all") listOf(SearchResultRow(
                    WorkingCopyView("PUBLIC-personal-database", "PUBLIC-personal-root", "/PUBLIC/search-personal"), 2uL,
                    if (language == UiLanguage.RUSSIAN) "Личная" else "Personal",
                    overview.groups.single { it.id == "PUBLIC-services" }.name,
                    EntryRow("PUBLIC-forum", "PUBLIC-services", "Studio Forum", "demo@studio.example", null, null, false, false, IconView.Default, 1_758_010_860_000))) else emptyList()
                SearchContent(if (case == "search-empty") "PUBLIC absent query" else "studio",
                    if (case == "search-all") SearchScope.ALL_UNLOCKED else SearchScope.CURRENT,
                    overview.name, if (case == "search-empty") emptyList() else rows, false, false, {}, {}, {}, {})
            }
        }
        "icon-picker" -> Box(Modifier.fillMaxSize()) {
            VisualSurface("edit-appearance", theme, language)
            IconSourceSheet(StoredFaviconView("PUBLIC-loaded-favicon", true), {}, {}, {}, {}, {}, {})
        }
        "lucide-picker" -> LucidePickerContent("key-round", {}, {})
        "image-url" -> IconUrlContent("https://studio.example/icon.png", {}, false, false, null, {}, {})
        "drawer" -> LibraryDrawer(remember { DrawerState(DrawerValue.Open) }, true, false, drawerContent = {
            DatabaseDrawer(localizedOverview(language), emptyList(), "PUBLIC-work", publicDrafts, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, trash = {}, received = {})
        }) { VisualSurface("entries", theme, language) }
        "generator", "generator-use", "passphrase" -> {
            val state = remember { GeneratorState(VisualGeneratorBackend).also { if (case == "passphrase") it.selectMode(GeneratorMode.PASSPHRASE) } }
            DisposableEffect(state) { onDispose { state.lock() } }
            GeneratorScreen(state, use = if (case == "generator-use") ({}) else null, copy = {})
        }
        "receive" -> ReceiveInvitationScreen(onJoin = { _, _ -> }, onClose = {})
        "invitation" -> InvitationScreen("TAYPEER-DEMO-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", if (language == UiLanguage.RUSSIAN) "Рабочая" else "Working", {}, {})
        "trash", "trash-selected" -> TrashScreen(publicTrash(language), localizedOverview(language).groups, null, false, false, { _, _ -> }, {}, {}, {}, {}, {})
        "clone", "move" -> ObjectActionScreen(ObjectActionSelection(FormKind.ENTRY, "PUBLIC-entry", if (case == "clone") ObjectAction.CLONE else ObjectAction.MOVE, "GitHub", "PUBLIC-services"), localizedOverview(language).groups, false, false, { _, _, _ -> }, {})
        "relay", "relay-custom" -> RelaySettingsContent(if (case == "relay-custom") RelayPreference.Custom("https://relay.example") else RelayPreference.Public, false, false, {}, {}, {})
        "drafts" -> LocalDraftsScreen(publicDrafts, true, resume = {}, discard = {}, close = {})
        "history-list" -> Scaffold(topBar = { PhoneBar(stringResource(R.string.history), back = {}) }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                HistoryListContent(publicHistory, emptySet(), true, true, {}, { _, _ -> }, {})
            }
        }
        "history-version", "history-version-additional", "history-version-appearance", "history-version-properties", "history-version-history" ->
            SavedRevisionScreen(publicHistory.last(), publicSavedEntry, null, false, true, {}, {}, {}, {}, {}, {},
                properties = publicSavedProperties,
                selectedTab = when (case) {
                    "history-version-additional" -> 1
                    "history-version-appearance" -> 2
                    "history-version-properties" -> 3
                    "history-version-history" -> 4
                    else -> 0
                }, revisions = publicHistory, additional = {
                    SavedAttributesContent(publicSavedAttributes, null, "", {}, {}, {})
                    AttachmentListContent(emptyList(), publicSavedEntry.id, publicHistory.last().id, false, true, { _, _, _, _ -> }, {}, {}, {})
                })
        "history-compare" -> HistoryComparisonScreen(publicSavedEntry, publicEntry, publicHistory.last(), null, null, false, false, true, {}, {}, {}, {}, {})
        "restore-version" -> RestoreVersionScreen(publicHistory.last(), localizedOverview(language).groups, false, false, null, false, {}, {}, initialDestination = publicSavedEntry.group)
        "change-password", "revoke" -> PasswordChangeScreen(case == "revoke", false, false, {}, {})
        "recover-control" -> RecoveryForm(if (language == UiLanguage.RUSSIAN) "Рабочая — восстановленная" else "Working — recovered",
            "PUBLIC visual recovery password", "PUBLIC visual recovery password", false, false, null, "PUBLIC Pixel 9", {}, {}, {}, {}, {}, {})
        "transfer-control" -> ManagementTransferForm(localizedOverview(language).name, "Pixel 9",
            listOf(ManagementRecipient("PUBLIC-visual-peer", "MacBook Pro")), "PUBLIC-visual-peer", false, false, null, null,
            null, false, false, {}, {}, {}, {})
        "accept-control" -> ManagementDecisionForm(localizedOverview(language).name, "MacBook Pro", "Pixel 9",
            ManagementHandoffView(publicManagementIntent, ManagementPhase.WAITING_DECISION, null), false, false, null,
            {}, {}, {}, {})
        "device-menu" -> Box(Modifier.fillMaxSize()) {
            VisualSurface("entries", theme, language)
            DeviceActionSheet("MacBook Pro", true, {}, {}, {}, {})
        }
        else -> Scaffold(contentWindowInsets = WindowInsets(0, 0, 0, 0), topBar = {
            val entry = case.startsWith("entry") || case.startsWith("edit")
            val metadata = case == "group-create" || case == "group-edit"
            LibraryHeader(when { metadata -> stringResource(if (case == "group-create") R.string.new_group else R.string.group); entry -> "GitHub"; case == "entries" -> if (language == UiLanguage.RUSSIAN) "Работа" else "Work"; case.startsWith("settings") -> stringResource(R.string.settings); else -> "Taypeer" }, case != "welcome", entry || metadata, case.startsWith("entry"), metadata, case.startsWith("settings"), {}, {}, {}, true, {})
        }, bottomBar = {
            if (case.startsWith("edit")) FormActions({}, {}, enabled = true)
            if (case == "group-create" || case == "group-edit") FormActions({}, {}, enabled = true,
                label = if (case == "group-create") R.string.create else R.string.save)
        }, floatingActionButton = {
            if (case == "entries") AddEntryFab(true) {}
        }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding)) {
                if (case.startsWith("entry") || case.startsWith("edit")) EntryTabs(when (case) { "entry-additional", "edit-additional" -> 1; "entry-appearance", "edit-appearance" -> 2; "entry-properties" -> 3; "entry-history" -> 4; else -> 0 }) {}
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    when (case) {
                        "welcome" -> WelcomeContent(listOf(WorkingCopyView("PUBLIC-working", "PUBLIC-root", "/PUBLIC/working")), {}, {}, {}, {})
                        "entries" -> EntriesContent(localizedOverview(language), "PUBLIC-work", "", {}, {}, {}, {})
                        "group-create", "group-edit" -> Column(Modifier.padding(16.dp)) {
                            val groups = localizedOverview(language).groups
                            val editing = case == "group-edit"
                            val selected = groups.single { it.id == "PUBLIC-infrastructure" }
                            val input = MetadataInput(MetadataEditor(
                                LocalForm("PUBLIC-group-form", 1uL, if (editing) FormKind.GROUP else FormKind.NEW_GROUP,
                                    if (editing) selected.id else null, false), selected.name,
                                if (language == UiLanguage.RUSSIAN) "Серверы и рабочие сервисы" else "Servers and work services",
                                if (editing) selected.icon else IconView.Default, if (editing) selected.parent else null,
                                (if (editing) groups.filter { it.id != selected.id } else groups).map { GroupParentCandidate(it.id, it.name) }))
                            MetadataFields(input, true, {}, {}, {}) {
                                IconPicker((input.source.icon as? IconView.Lucide)?.key, {},
                                    preview = savedIconDrawable(input.source.icon, R.drawable.ic_folder))
                            }
                        }
                        "entry" -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                            EntrySummary(publicEntry, listOf("work", "development"), null, null, {}, {}, {})
                        }
                        "edit" -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                            EntryFields(publicInput, true, "", { _, _ -> }, {}, {}, {}, {}, {}, {})
                        }
                        "settings", "settings-database" -> SettingsContent(theme, language, {}, {}, case == "settings-database", {}, {}, {}, relay = {}, devicePreferences = {
                            DevicePreferencesContent(DeviceSettingsView(300u, 30u, "PUBLIC Android"), false, false, {}, {}, {})
                        }, databaseName = localizedOverview(language).name, exchange = {}, passwordChange = {}, relayPreference = RelayPreference.Public,
                            deviceName = { DeviceNameContent("PUBLIC Android", null, {}, {}, {}) }, limits = {},
                            recovery = {}, saveCopy = {})
                        "entry-additional", "edit-additional" -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                            if (case == "entry-additional") {
                                Text(stringResource(R.string.attributes), style = MaterialTheme.typography.titleMedium)
                                EntryAttributesContent(publicSavedAttributes, null, "", {}, {}, {})
                            } else {
                                publicSavedAttributes.forEach { attribute ->
                                    EditableAttributeRow(attribute, attribute.value.orEmpty(), attribute.protected, true, {}, {}, {}, {}, {})
                                }
                                TextButton({}) { Text(stringResource(R.string.add_attribute)) }
                            }
                            AttachmentListContent(emptyList(), publicEntry.id, null, case == "edit-additional", true, { _, _, _, _ -> }, {}, {}, {})
                        }
                        "entry-appearance" -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) { AppearanceSummary(publicAppearance, "GitHub") }
                        "edit-appearance" -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) { AppearanceEditor(publicAppearance, {}, {}, {}, {}, "GitHub") }
                        "entry-properties" -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) { EntryPropertiesContent(publicProperties, {}) }
                        "entry-history" -> Column(Modifier.padding(16.dp)) { EntryHistoryContent(publicHistory, true, publicHistory.last().id) { _, _ -> } }
                    }
                }
            }
        }
    }
}

private val publicEntry = EntryRow("PUBLIC-entry", "PUBLIC-work", "GitHub", "demo@studio.example", "https://github.com", "PUBLIC design fixture.\nNo real credentials.", true, false, IconView.Default, 1_758_010_860_000)
private val publicSavedEntry = publicEntry.copy(notes = "PUBLIC previous notes.\nNo real credentials.")
/** Synthetic numeric composition only; actual measurements and writes have real-worker tests. */
private val publicPolicy = PolicyDetailsView(
    PolicyView(10uL * 1048576uL, 100uL * 1048576uL, 1000u),
    DatabaseUsageView(2_516_582uL, 100uL * 1048576uL, false, 3_145_728uL, 0uL, 4_194_304uL, false, false),
    KdfParametersView(KdfAlgorithm.ARGON2ID, 65536u, 1u, 5u), 1200uL,
)
/** Public native-shaped snapshots test composition; restoration correctness is exercised separately. */
private fun publicMetadataRevision(kind: FormKind, language: UiLanguage): MetadataRevisionView {
    val russian = language == UiLanguage.RUSSIAN
    val overview = localizedOverview(language)
    val saved = if (kind == FormKind.GROUP) MetadataSnapshotView(
        if (russian) "Серверы" else "Servers",
        if (russian) "Рабочие серверы и сервисы" else "Work servers and services",
        "PUBLIC-work", IconView.Lucide("server")) else MetadataSnapshotView(
        if (russian) "Студия" else "Studio",
        if (russian) "Рабочие проекты студии" else "Studio work projects", null, null)
    val current = if (kind == FormKind.GROUP) MetadataSnapshotView(
        overview.groups.single { it.id == "PUBLIC-infrastructure" }.name,
        if (russian) "Инфраструктура и облачные сервисы" else "Infrastructure and cloud services",
        "PUBLIC-services", IconView.Lucide("folder")) else MetadataSnapshotView(overview.name,
        if (russian) "Рабочие сервисы и учётные записи" else "Work services and accounts", null, null)
    return MetadataRevisionView(kind, if (kind == FormKind.GROUP) "PUBLIC-infrastructure" else null,
        "PUBLIC-metadata-revision", 1700000000123L, saved, current, listOf("1".repeat(64)),
        if (kind == FormKind.GROUP) overview.groups.filter { it.id != "PUBLIC-infrastructure" }
            .map { GroupParentCandidate(it.id, it.name) } else emptyList())
}
private val publicManagementIntent = ManagementIntentView("PUBLIC-database", "PUBLIC-root", "PUBLIC-control",
    "PUBLIC-visual-transfer", "PUBLIC-visual-manager", "PUBLIC-visual-peer")
private val publicInput = EntryInput(EntryEditor("PUBLIC-database", 1uL, LocalForm("PUBLIC-draft", 1uL, FormKind.ENTRY, "PUBLIC-entry", false), "PUBLIC-entry", "PUBLIC-work", "GitHub", "demo@studio.example", "https://github.com", "PUBLIC design fixture.\nNo real credentials.", listOf("work", "development"), true, null, null, emptyList()))
private val publicAppearance = AppearanceView("key-round", null, null, null)
private val publicProperties = EntryProperties("PUBLIC-entry", 1_757_928_720_000, 1_758_010_860_000, null, listOf("work", "development"), publicAppearance, null)
private val publicSavedProperties = EntryProperties("PUBLIC-entry", 1_757_928_720_000, 1_757_928_720_000,
    1_794_580_920_000, listOf("work", "saved-version"), AppearanceView(lucide = "server", image = null, foreground = 0xDCE2EAFFu, background = 0x2A2E36FFu), null)
private val publicSavedAttributes = listOf(
    AttributeRow("PUBLIC-environment", "Environment", "PUBLIC test environment", false),
    AttributeRow("PUBLIC-token", "Saved protected token", null, true),
)
private val publicHistory = listOf(HistoryRow("PUBLIC-revision-2", "GitHub", null, 1_758_010_860_000), HistoryRow("PUBLIC-revision-1", "GitHub", null, 1_757_928_720_000))
private fun publicTrash(language: UiLanguage) = listOf(
    TrashRow(FormKind.ENTRY, "PUBLIC-old-server", "PUBLIC-generation-1", if (language == UiLanguage.RUSSIAN) "Старый сервер" else "Old server", "PUBLIC-work"),
    TrashRow(FormKind.GROUP, "PUBLIC-archive", "PUBLIC-generation-2", if (language == UiLanguage.RUSSIAN) "Архив проектов" else "Project archive", null),
)
private val publicDrafts = listOf(
    LocalForm("PUBLIC-entry-draft", 2uL, FormKind.ENTRY, "PUBLIC-entry", true),
    LocalForm("PUBLIC-new-entry-draft", 1uL, FormKind.NEW_ENTRY, null, true),
    LocalForm("PUBLIC-group-draft", 3uL, FormKind.GROUP, "PUBLIC-infrastructure", true),
)

/** Fixed native-shaped results keep screenshots deterministic; native generation has real-worker tests. */
private object VisualGeneratorBackend : GeneratorBackend {
    override suspend fun password(options: PasswordOptions) = GeneratedSecretView("PUBLIC".repeat(5), 196.64, GeneratedQuality.HIGH)
    override suspend fun passphrase(words: UByte, separator: String) = GeneratedSecretView("public-amber-canyon-copper-meadow-violet", 77.55, GeneratedQuality.HIGH)
}

private fun localizedOverview(language: UiLanguage): DocumentOverview {
    if (language == UiLanguage.ENGLISH) return publicOverview
    val names = mapOf("PUBLIC-work" to "Работа", "PUBLIC-services" to "Сервисы", "PUBLIC-infrastructure" to "Инфраструктура", "PUBLIC-personal" to "Личное", "PUBLIC-archive" to "Архив")
    return publicOverview.copy(name = "Рабочая", groups = publicGroups.map { it.copy(name = names.getValue(it.id)) })
}
