package dev.taypeer

import android.net.Uri
import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.BackHandler
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.background
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import android.view.ViewTreeObserver
import android.content.Context
import android.content.ContextWrapper
import dev.taypeer.bridge.*
import dev.taypeer.platform.ExchangeScheduler
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

private sealed interface Authentication {
    data object Create : Authentication
    data class Open(val copy: WorkingCopyView) : Authentication
    data class Import(val uri: Uri) : Authentication
    data class Recovery(val copy: WorkingCopyView, val operation: String, val candidate: Boolean) : Authentication
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LibraryScreen(state: DocumentState, attachments: AttachmentActions, copySecret: (String) -> Boolean = { false }, copy: (String) -> Unit) {
    val chooseCopyLocation = rememberCiphertextCopyPicker(state.databaseCopy)
    if (state.databaseCopy.showing) CiphertextCopyScreen(state.databaseCopy, chooseCopyLocation)
    else key(state.secrecyEpoch) { LibraryContent(state, attachments, copySecret, copy) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LibraryContent(state: DocumentState, attachments: AttachmentActions, copySecret: (String) -> Boolean, copy: (String) -> Unit) {
    var authentication by remember { mutableStateOf<Authentication?>(state.lockedCopy?.let(Authentication::Open)) }
    var join by remember { mutableStateOf(false) }
    var resumeJoin by remember { mutableStateOf<String?>(null) }
    var forms by remember { mutableStateOf(false) }
    var settings by remember { mutableStateOf(false) }
    var relaySettings by remember { mutableStateOf(false) }
    var relayCustom by remember { mutableStateOf(false) }
    var historyLaunch by remember { mutableStateOf<HistoryLaunch?>(null) }
    var passwordChange by remember { mutableStateOf(false) }
    var actions by remember { mutableStateOf(false) }
    var searchScope by remember(state.contextSessionEpoch) { mutableStateOf(false) }
    var sorting by remember(state.contextSessionEpoch) { mutableStateOf(false) }
    val contextEpoch = state.contextSessionEpoch
    val contextDatabase = state.overview?.database
    val contextGeneration = state.overview?.generation
    // Only exact target IDs survive while a menu is open; labels come from the live view.
    var groupActions by remember(contextEpoch, contextDatabase, contextGeneration) { mutableStateOf<String?>(null) }
    var entryActions by remember(contextEpoch, contextDatabase, contextGeneration) { mutableStateOf<String?>(null) }
    // An empty welcome drawer has coincident anchors. A newly opened database
    // needs a fresh closed state instead of inheriting its previous zero offset.
    val drawer = remember(state.overview?.database) { DrawerState(DrawerValue.Closed) }
    val scope = rememberCoroutineScope()
    val context = androidx.compose.ui.platform.LocalContext.current
    val application = context.applicationContext as TaypeerApplication
    val biometryForeground = biometryForeground()
    val relayState = remember(application, scope) { RelaySettingsState(application.host, scope) }
    LaunchedEffect(settings) { if (settings) relayState.load() }
    LaunchedEffect(settings, state.overview?.database, state.catalog, biometryForeground) {
        if (settings && biometryForeground) state.loadBiometry()
    }
    LaunchedEffect(state.history) { if (historyLaunch?.selection !== state.history) historyLaunch = null }
    fun openHistory(entry: String, row: HistoryRow, action: HistoryAction) {
        state.showHistory(FormKind.ENTRY, entry, if (action == HistoryAction.PURGE) null else row) { selection ->
            if (selection.rows.any { it.id == row.id }) historyLaunch = HistoryLaunch(selection, row.id, action)
        }
    }
    val notificationPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { state.openExchangeAfterNavigation() }
    fun exchange() {
        state.leaveEditor {
            if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
                notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            else state.openExchangeAfterNavigation()
        }
    }
    fun closeDrawer(action: () -> Unit) { scope.launch { drawer.close(); action() } }
    fun contextCurrent() = contextDatabase != null && state.contextSessionEpoch == contextEpoch &&
        state.overview?.database == contextDatabase && state.overview?.generation == contextGeneration && !state.opening
    fun openGroupActions(group: String) {
        closeDrawer {
            if (contextCurrent() && !state.navigating && state.overview?.groups?.any { it.id == group } == true) groupActions = group
        }
    }
    LaunchedEffect(state.navigating, state.opening, state.activeForm?.draft, state.inspectedEntry?.id,
        state.history, state.exchange, state.policySettings.page, settings, forms, state.searches.showing, state.lifecycle.selection,
        state.lifecycle.showingTrash, state.management.route, state.recovery.showing, state.metadataHistory.showing, state.groupFavicons.showing) {
        entryActions = null; groupActions = null
    }
    fun resumeRecovery(row: PendingRecoveryView) {
        if (row.phase == RecoveryPhase.REFUSED_BEFORE_PUBLICATION) {
            state.recovery.showRefusal(row); authentication = null
        } else authentication = Authentication.Recovery(row.source, row.operation,
            candidate = row.phase == RecoveryPhase.CANDIDATE_PUBLISHED || row.phase == RecoveryPhase.PROOF_CONFIRMED)
    }
    LaunchedEffect(state.databaseImport) { state.databaseImport?.let { authentication = Authentication.Import(it) } }
    BackHandler(enabled = drawer.isOpen) { scope.launch { drawer.close() } }
    BackHandler(enabled = settings) { settings = false }
    BackHandler(enabled = state.searches.showing && state.entry == null && state.inspectedEntry == null && !settings) { state.search("") }
    BackHandler(enabled = state.history == null && state.exchange == null && !settings && authentication == null && !join && (state.entry != null || state.metadata != null || state.inspectedEntry != null)) {
        if (state.searches.returning != null) state.returnToSearch() else state.chooseGroup(state.selectedGroup)
    }
    authentication?.let { auth ->
        key(auth) { AuthenticationDialog(state, auth, {
            if (auth is Authentication.Import) state.selectDatabaseImport(null)
            state.chooseAnotherDatabase(); authentication = null
        }, resume = { row ->
            if (row.phase == RecoveryPhase.REFUSED_BEFORE_PUBLICATION) {
                state.recovery.showRefusal(row); state.chooseAnotherDatabase(); authentication = null
            } else authentication = Authentication.Recovery(row.source, row.operation,
                candidate = row.phase == RecoveryPhase.CANDIDATE_PUBLISHED || row.phase == RecoveryPhase.PROOF_CONFIRMED)
        }) }
        return
    }
    if (join || resumeJoin != null) {
        ReceiveInvitationScreen(request = resumeJoin, onJoin = { code, password ->
            if (resumeJoin == null) state.join(code, password) else state.resumeJoin(code, password)
            join = false; resumeJoin = null
        }, onClose = { join = false; resumeJoin = null })
        return
    }
    if (state.recovery.showing) {
        RecoveryScreen(state.recovery, application.deviceSettings.settings?.deviceName) { target ->
            authentication = Authentication.Recovery(target.source, target.operation, candidate = false)
        }
        return
    }
    if (state.management.route != null) {
        ManagementScreen(state.management, state.overview?.name,
            application.deviceSettings.settings?.deviceName, state.overview?.managing == true)
        return
    }
    if (relaySettings) { RelaySettingsContent(relayState.preference, relayState.busy, relayState.failed,
        relayState::apply, relayState::load, { relaySettings = false }, startCustom = relayCustom); return }
    if (passwordChange) { PasswordRotationScreen(state, null) { passwordChange = false }; return }
    if (forms) {
        LocalDraftsScreen(state.forms, state.overview?.writable == true, state.navigating,
            resume = { state.resumeForm(it); forms = false }, discard = state::deleteForm, close = { forms = false }, error = state.error)
        return
    }
    if (state.received.showing) {
        ReceivedSourcesScreen(state.received, state.overview?.groups.orEmpty(), state.overview?.writable == true, copy) {
            if (it.kind == FormKind.ENTRY) state.selectEntry(it.target) else state.chooseGroup(it.target)
        }
        return
    }
    if (state.metadataHistory.showing) {
        MetadataHistoryScreen(state.metadataHistory, state.overview?.writable == true, copy, blocked = state.editingBlocked)
        return
    }
    state.history?.let { selection ->
        SecretAccessContainer(state.accesses, state.overview?.database) {
            HistoryScreen(state, attachments, selection, copy, copySecret, historyLaunch) { historyLaunch = null }
        }
        return
    }
    state.exchange?.let { ExchangeScreen(state, it, copy); return }
    if (state.policySettings.page != null) { PolicySettingsScreen(state.policySettings, state.overview?.name); return }
    if (state.groupFavicons.showing) { GroupFaviconsScreen(state.groupFavicons); return }
    val lifecycle = state.lifecycle
    lifecycle.selection?.let { selection ->
        ObjectActionScreen(selection, state.overview?.groups.orEmpty(), lifecycle.busy, lifecycle.error,
            lifecycle::confirmAction, lifecycle::closeAction, lifecycle.attempt, state.overview?.writable == true)
        return
    }
    if (lifecycle.showingTrash) {
        TrashScreen(lifecycle.rows, state.overview?.groups.orEmpty(), lifecycle.review, lifecycle.busy, lifecycle.error,
            lifecycle::prepareRestore, lifecycle::preparePurge, lifecycle::confirmReview, lifecycle::cancelReview,
            lifecycle::refreshTrash, lifecycle::closeTrash, state.overview?.writable == true)
        return
    }
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(state.activeForm?.draft) {
        if (state.activeForm == null) { focus.clearFocus(force = true); keyboard?.hide() }
    }
    val editor = state.entry != null || state.inspectedEntry != null || state.metadata != null
    LibraryDrawer(drawer, state.overview != null, gesturesEnabled = !editor && state.overview != null && !settings, drawerContent = {
        state.overview?.let { overview ->
                DatabaseDrawer(overview, state.catalog, state.selectedGroup, state.forms,
                    { group -> closeDrawer { state.chooseGroup(group) } },
                    ::openGroupActions,
                    { closeDrawer { state.beginMetadata(FormKind.NEW_GROUP, parent = null) } },
                    { selected -> closeDrawer { state.chooseDatabase(selected) { authentication = Authentication.Open(it) } } },
                    { closeDrawer { authentication = Authentication.Create } },
                    { closeDrawer(attachments.importDatabase) },
                    { closeDrawer { join = true } },
                    { closeDrawer(::exchange) },
                    { closeDrawer { forms = true } },
                    { closeDrawer { state.leaveEditor { settings = true } } },
                    { closeDrawer(state::lockCurrent) }, trash = { closeDrawer(lifecycle::openTrash) }, received = { closeDrawer(state.received::open) }, groupIcon = { group, fallback ->
                        if (group.icon is IconView.Image) DocumentIcon(state, IconScope.Group(group.id), group.icon, fallback) else Glyph(fallback)
                    }, unlockedCopies = state.unlockedCopies)
        }
    }) {
        Scaffold(
            topBar = {
                if (state.searches.showing && !editor && !settings) PhoneBar(stringResource(R.string.search), back = { state.search("") }) {
                    IconAction(R.drawable.ic_ellipsis, R.string.sort_entries, Modifier.testTag("sort-search")) { sorting = true }
                    IconAction(R.drawable.ic_x, R.string.clear_search, Modifier.testTag("clear-search-header")) { state.search("") }
                } else {
                val title = when {
                    settings -> stringResource(R.string.settings)
                    state.entry != null -> state.entry!!.source.entry?.let { state.entry!!.title } ?: stringResource(R.string.new_entry)
                    state.inspectedEntry != null -> state.inspectedEntry!!.title
                    state.metadata != null -> stringResource(formText(state.metadata!!.source.form.kind))
                    else -> state.overview?.groups?.firstOrNull { it.id == state.selectedGroup }?.name ?: state.overview?.name ?: stringResource(R.string.app_name)
                }
                LibraryHeader(title, state.overview != null, editor, state.inspectedEntry != null, state.metadata != null, settings,
                    back = { if (settings) settings = false else if (state.searches.returning != null) state.returnToSearch() else state.chooseGroup(state.selectedGroup) },
                    menu = { scope.launch { drawer.open() } }, edit = { state.inspectedEntry?.let { state.beginEntry(it.id) } },
                    editEnabled = state.overview?.writable == true, openActions = { actions = true }) {
                            DropdownMenu(actions, { actions = false }) {
                                DropdownMenuItem({ Text(stringResource(R.string.generator_short)) }, { actions = false; state.generator = true }, Modifier.testTag("open-generator"))
                                DropdownMenuItem({ Text(stringResource(R.string.settings)) }, { actions = false; state.leaveEditor { settings = true } }, Modifier.testTag("open-settings"))
                                val selectedEntry = state.inspectedEntry ?: state.entry?.source?.entry?.let { id -> state.overview?.entries?.firstOrNull { it.id == id } }
                                if (selectedEntry != null) {
                                    DropdownMenuItem({ Text(stringResource(R.string.history)) }, { actions = false; state.showHistory(FormKind.ENTRY, selectedEntry.id) }, Modifier.testTag("open-entry-history"))
                                    DropdownMenuItem({ Text(stringResource(R.string.clone_entry)) }, { actions = false; lifecycle.openAction(ObjectActionSelection(FormKind.ENTRY, selectedEntry.id, ObjectAction.CLONE, selectedEntry.title, selectedEntry.group)) }, Modifier.testTag("clone-entry"), enabled = state.overview?.writable == true)
                                    DropdownMenuItem({ Text(stringResource(R.string.move_entry)) }, { actions = false; lifecycle.openAction(ObjectActionSelection(FormKind.ENTRY, selectedEntry.id, ObjectAction.MOVE, selectedEntry.title, selectedEntry.group)) }, Modifier.testTag("move-entry"), enabled = state.overview?.writable == true)
                                    DropdownMenuItem({ Text(stringResource(R.string.trash)) }, { actions = false; state.trash(FormKind.ENTRY, selectedEntry.id, selectedEntry.group) }, Modifier.testTag("trash-entry"), enabled = state.overview?.writable == true)
                                }
                                if (state.overview != null) {
                                    DropdownMenuItem({ Text(stringResource(R.string.save_database_copy)) }, { actions = false; state.beginDatabaseCopy() }, Modifier.testTag("save-database-copy"))
                                    DropdownMenuItem({ Text(stringResource(R.string.sync)) }, { actions = false; exchange() }, Modifier.testTag("open-exchange"))
                                    DropdownMenuItem({ Text(stringResource(R.string.trash_title)) }, { actions = false; lifecycle.openTrash() }, Modifier.testTag("open-trash"))
                                    DropdownMenuItem({ Text(stringResource(R.string.sort_entries)) }, { actions = false; sorting = true }, Modifier.testTag("sort-entries"))
                                    DropdownMenuItem({ Text(stringResource(R.string.lock)) }, { actions = false; state.lockCurrent() }, Modifier.testTag("lock-database"))
                                    DropdownMenuItem({ Text(stringResource(R.string.recover_management)) }, { actions = false; state.beginRecovery(::resumeRecovery) }, Modifier.testTag("recover-management"))
                                }
                            }
                }
                }
            },
            floatingActionButton = {
                if (state.overview != null && !editor && !settings && !state.searches.showing) {
                    val enabled = state.overview?.writable == true && state.overview!!.groups.any { it.id == state.selectedGroup }
                    AddEntryFab(enabled) { state.beginEntry() }
                }
            },
            bottomBar = {
                if (state.activeForm != null && !settings) FormActions(state::cancelForm, state::saveNow,
                    enabled = !state.editingBlocked && state.saveLabel != SaveLabel.SAVING && state.overview?.writable == true,
                    label = if (state.activeForm?.kind == FormKind.NEW_ENTRY || state.activeForm?.kind == FormKind.NEW_GROUP) R.string.create else R.string.save,
                    cancelEnabled = !state.editingBlocked, status = {
                        if (state.saveLabel == SaveLabel.SAVING || state.saveLabel == SaveLabel.FAILED) Text(stringResource(saveText(state.saveLabel)), Modifier.padding(horizontal = 16.dp, vertical = 4.dp).testTag("save-status"), style = MaterialTheme.typography.bodySmall)
                    })
            },
        ) { padding ->
            val route = when { settings -> "settings"; state.overview == null -> "welcome"; state.metadata != null -> "metadata"; state.entry != null -> "entry"; state.inspectedEntry != null -> "inspected"; state.searches.showing -> "search"; else -> "entries" }
            Column(Modifier.fillMaxSize().padding(padding).testTag("route-$route")) {
                if (state.entry != null || state.inspectedEntry != null) EntryTabs(state.entryTab, state::selectTab)
                Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                    if (state.searches.returning != null && editor && !settings) FlatAction(R.string.return_search_results,
                        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("return-search-results"),
                        enabled = !state.editingBlocked, ghost = true, action = state::returnToSearch)
                    SecretAccessStatus(state.accesses, state.overview?.database ?: state.lockedCopy?.database)
                    ManagementHandoffs(state.management, state::reviewManagement)
                    if (!editor && !settings) PendingRecoveries(state.recovery) { row ->
                        state.leaveEditor { resumeRecovery(row) }
                    }
                    if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp).testTag("document-error"))
                    if (state.invalidInput) Text(stringResource(R.string.invalid_form), color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp).testTag("invalid-form"))
                    PendingAttachment(state)
                    PendingIcon(state)
                    if (state.opening || state.navigating) LinearProgressIndicator(Modifier.fillMaxWidth())
                    if (state.overview != null && !editor && !settings) UnconfirmedOperations(state)
                    if (state.overview != null && !editor && !settings) MetadataRestoreRetries(state.metadataHistory,
                        enabled = !state.navigating && state.overview?.writable == true)
                    when {
                        settings -> SettingsContent(LocalUiPreferences.current.theme, LocalUiPreferences.current.language, LocalUiPreferences.current::selectTheme, LocalUiPreferences.current::selectLanguage, state.overview != null, {
                            settings = false; state.beginMetadata(FormKind.DATABASE)
                        }, { settings = false; state.showPolicy() }, { state.generator = true }, relay = { relayCustom = true; relaySettings = true },
                            devicePreferences = { DevicePreferenceFields((context.applicationContext as TaypeerApplication).deviceSettings) },
                            databaseName = state.overview?.name, writable = state.overview?.writable == true, managing = state.overview?.managing == true,
                            exchange = { settings = false; exchange() }, passwordChange = { state.clearError(); settings = false; passwordChange = true },
                            relayPreference = relayState.preference, relayBusy = relayState.busy, relayFailed = relayState.failed,
                            selectRelay = relayState::apply, reloadRelay = relayState::load,
                            deviceName = { DeviceNameField(application.deviceSettings) },
                            recovery = { settings = false; state.beginRecovery(::resumeRecovery) },
                            saveCopy = { settings = false; state.beginDatabaseCopy() },
                            biometry = { BiometrySettings(state.biometry, state::enableBiometry, state::disableBiometry, state::retryBiometry) },
                            limits = { settings = false; state.showPolicy(PolicyPage.LIMITS) })
                        state.overview == null -> {
                            WelcomeContent(state.catalog, { authentication = Authentication.Create }, attachments.importDatabase, { join = true }, { copy -> state.chooseDatabase(copy) { authentication = Authentication.Open(it) } })
                            state.pending.forEach { request -> Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                                Text(stringResource(R.string.waiting_approval), Modifier.weight(1f))
                                TextButton({ resumeJoin = request.request }) { Text(stringResource(R.string.continue_action)) }
                            } }
                        }
                        state.inspectedEntry != null -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) { EntryDetails(state, attachments, copy, copySecret, ::openHistory) }
                        state.entry != null -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) { EntryForm(state, attachments, copy, copySecret, ::openHistory) }
                        state.metadata != null -> Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) { MetadataForm(state, attachments) }
                        state.searches.showing -> SearchContent(state.searches.query, state.searches.scope, state.searches.currentName,
                            state.searches.results, state.searches.busy, state.searches.error, state::search, { searchScope = true },
                            state.searches::refresh, state::selectSearchResult, preview = { row ->
                                val fallback = savedIconDrawable(row.entry.icon, R.drawable.ic_key_round)
                                if (row.entry.icon is IconView.Image) SearchIcon(state.searches, row, fallback) else Glyph(fallback)
                            })
                        else -> EntriesContent(state.overview!!, state.selectedGroup, "", state::search, { state.beginMetadata(FormKind.NEW_GROUP, parent = null) }, { state.beginEntry() }, state::selectEntry, state.entrySort, iconPreview = { row ->
                            val fallback = savedIconDrawable(row.icon, R.drawable.ic_key_round)
                            if (row.icon is IconView.Image) DocumentIcon(state, IconScope.Entry(row.id), row.icon, fallback) else Glyph(fallback)
                        }, contextActions = { id ->
                            if (contextCurrent() && !state.navigating && state.overview?.entries?.any { it.id == id } == true) entryActions = id
                        }, searchScope = { SearchScopeControl(state.searches.scope, state.overview?.name) { searchScope = true } })
                    }
                    state.undo?.let { Snackbar(action = { TextButton(state::undoTrash) { Text(stringResource(R.string.undo)) } }) { Text(stringResource(R.string.moved_to_trash)) } }
                }
            }
        }
    }
    if (searchScope && contextCurrent() && !editor && !settings) SearchScopeSheet(state.searches.scope,
        select = { searchScope = false; state.selectSearchScope(it) }, close = { searchScope = false })
    if (sorting && contextCurrent() && !settings) EntrySortSheet(state.entrySort,
        select = { sorting = false; state.selectEntrySort(it) }, close = { sorting = false })
    entryActions?.takeIf { contextCurrent() }?.let { id -> state.overview?.entries?.firstOrNull { it.id == id }?.let { entry ->
        EntryActionsSheet(entry, state.overview?.writable == true, close = { entryActions = null },
            edit = { entryActions = null; if (contextCurrent()) state.beginEntry(entry.id) },
            clone = { entryActions = null; if (contextCurrent()) lifecycle.openAction(ObjectActionSelection(FormKind.ENTRY, entry.id, ObjectAction.CLONE, entry.title, entry.group)) },
            move = { entryActions = null; if (contextCurrent()) lifecycle.openAction(ObjectActionSelection(FormKind.ENTRY, entry.id, ObjectAction.MOVE, entry.title, entry.group)) },
            history = { entryActions = null; if (contextCurrent()) state.showHistory(FormKind.ENTRY, entry.id) },
            trash = { entryActions = null; if (contextCurrent()) state.trash(FormKind.ENTRY, entry.id, entry.group) })
    } }
    groupActions?.takeIf { contextCurrent() }?.let { id -> state.overview?.groups?.firstOrNull { it.id == id }?.let { group ->
        GroupActionsSheet(group, state.overview?.writable == true, close = { groupActions = null },
            create = { groupActions = null; if (contextCurrent()) state.beginMetadata(FormKind.NEW_GROUP, parent = group.id) },
            edit = { groupActions = null; if (contextCurrent()) state.beginMetadata(FormKind.GROUP, group.id) },
            clone = { groupActions = null; if (contextCurrent()) lifecycle.openAction(ObjectActionSelection(FormKind.GROUP, group.id, ObjectAction.CLONE, group.name, group.parent)) },
            move = { groupActions = null; if (contextCurrent()) lifecycle.openAction(ObjectActionSelection(FormKind.GROUP, group.id, ObjectAction.MOVE, group.name, group.parent)) },
            history = { groupActions = null; if (contextCurrent()) state.showHistory(FormKind.GROUP, group.id) },
            favicons = { groupActions = null; if (contextCurrent()) state.beginGroupFavicons(group.id) },
            trash = { groupActions = null; if (contextCurrent()) state.trash(FormKind.GROUP, group.id, group.parent) })
    } }
    if (state.navigationDecision) AlertDialog(
        onDismissRequest = state::stay, modifier = Modifier.testTag("unsaved-changes"),
        title = { Text(stringResource(R.string.unsaved_changes)) },
        text = { Column {
            Text(stringResource(R.string.unsaved_changes_message))
            if (state.invalidInput) Text(stringResource(R.string.invalid_form), color = MaterialTheme.colorScheme.error)
            if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
        } },
        confirmButton = { TextButton(state::saveNavigation, enabled = !state.navigating, modifier = Modifier.testTag("navigation-save")) { Text(stringResource(R.string.save)) } },
        dismissButton = { Column {
            TextButton(state::discardNavigation, enabled = !state.navigating, modifier = Modifier.testTag("navigation-discard")) { Text(stringResource(R.string.do_not_save)) }
            TextButton(state::stay, enabled = !state.navigating, modifier = Modifier.testTag("navigation-stay")) { Text(stringResource(R.string.stay)) }
        } },
    )
}

@Composable
private fun EntryDetails(state: DocumentState, attachments: AttachmentActions, copy: (String) -> Unit, copySecret: (String) -> Boolean, historyAction: (String, HistoryRow, HistoryAction) -> Unit) {
    val entry = state.inspectedEntry ?: return
    if (state.entryTab == 2) { AppearanceSummary(state.appearance, entry.title, previewContent = {
        if (state.appearance?.image != null) DocumentIcon(state, IconScope.Entry(entry.id), state.appearance?.image, appearanceIconDrawable(state.appearance))
        else Glyph(appearanceIconDrawable(state.appearance))
    }); return }
    if (state.entryTab == 3) { PropertiesContent(state.properties, copy); return }
    if (state.entryTab == 4) { HistoryContent(state, entry.id, historyAction); return }
    if (state.entryTab == 1) {
        Text(stringResource(R.string.attributes), style = MaterialTheme.typography.titleMedium)
        EntryAttributesContent(state.inspectedAttributes, state.revealedAttribute, state.revealed,
            { state.reveal(it) }, { state.copySecret(copySecret, it) }, copy, display = { state.secretDisplay })
        Attachments(state, attachments, state.attachments, entry.id)
        return
    }
    EntrySummary(entry, state.properties?.tags.orEmpty(), state.properties?.expiresAt,
        if (state.revealed.isNotEmpty() && state.revealedAttribute == null) state.revealed else null,
        { if (state.revealed.isEmpty()) state.reveal() else state.hideSecrets() }, copy, { state.copySecret(copySecret) }, display = state.secretDisplay)
}

@Composable
private fun AuthenticationDialog(state: DocumentState, authentication: Authentication, dismiss: () -> Unit, resume: (PendingRecoveryView) -> Unit) {
    var name by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var repeat by remember { mutableStateOf("") }
    var revealed by remember(LocalSecretMaskEpoch.current) { mutableStateOf(false) }
    val operation = remember { newOperation() }
    val creating = authentication == Authentication.Create
    var attempted by remember { mutableStateOf(false) }
    var biometricAttempted by remember { mutableStateOf(false) }
    var recoveryAttempted by remember { mutableStateOf(authentication is Authentication.Recovery) }
    val busy = state.opening || state.recovery.busy
    val biometryForeground = biometryForeground()
    LaunchedEffect(authentication, biometryForeground) {
        if (authentication is Authentication.Open && biometryForeground) state.loadBiometry(authentication.copy)
    }
    val retained = if (authentication is Authentication.Open && !state.recovery.pendingFailure)
        state.recovery.pending.firstOrNull { it.source.path == authentication.copy.path } else null
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    fun close() { focus.clearFocus(force = true); keyboard?.hide(); dismiss() }
    fun cancel() { if (!busy) { if (recoveryAttempted) state.recovery.close(); close() } }
    BackHandler { cancel() }
    LaunchedEffect(state.overview, state.opening, state.recovery.busy, state.recovery.failure) {
        if (attempted && !busy) {
            if (recoveryAttempted && state.recovery.showing && state.recovery.failure != RecoveryFailure.SOURCE_PASSWORD) close()
            else if (!recoveryAttempted && state.overview != null && !state.error) close()
        }
    }
        Surface(Modifier.fillMaxSize().testTag("route-authentication")) {
            Scaffold(topBar = { PhoneBar(stringResource(if (creating) R.string.create_database else if (authentication is Authentication.Recovery) R.string.recover_management else R.string.unlock), back = ::cancel) },
                bottomBar = { FormActions(::cancel, {
                    focus.clearFocus(force = true); keyboard?.hide()
                    attempted = true
                    when (authentication) {
                        Authentication.Create -> state.create(name, description.ifEmpty { null }, password, operation)
                        is Authentication.Open -> state.unlock(authentication.copy, password)
                        is Authentication.Import -> state.import(authentication.uri, password)
                        is Authentication.Recovery -> state.authenticateRecovery(authentication.copy, password, authentication.operation)
                    }
                }, enabled = !busy && password.isNotEmpty() && (!creating || (password == repeat && name.isNotBlank())), tag = "authenticate", label = if (creating) R.string.create else if (authentication is Authentication.Recovery) R.string.continue_action else R.string.unlock, cancelEnabled = !busy) }) { padding ->
                Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    if (authentication is Authentication.Open) SecretAccessStatus(state.accesses, authentication.copy.database)
                    if (authentication is Authentication.Open && state.management.database == authentication.copy.database)
                        ManagementHandoffs(state.management, state::reviewManagement)
                    if (creating) {
                        PlainField(name, R.string.name, "database-name", update = { name = it })
                        PlainField(description, R.string.description, "database-description", singleLine = false, update = { description = it })
                    } else {
                        Glyph(R.drawable.ic_lock)
                        Text(stringResource(R.string.database_locked), Modifier.testTag("database-locked"), style = MaterialTheme.typography.titleMedium)
                        Text(stringResource(R.string.working_copy), color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
                    }
                    PlainField(password, if (authentication is Authentication.Recovery && authentication.candidate) R.string.recovery_resume_password else R.string.master_password, "master-password", enabled = !busy, transformation = if (revealed && !busy) VisualTransformation.None else PasswordVisualTransformation(), keyboard = KeyboardOptions(keyboardType = KeyboardType.Password), actions = {
                        IconAction(if (revealed) R.drawable.ic_eye_off else R.drawable.ic_eye, if (revealed) R.string.hide else R.string.reveal, Modifier.testTag("reveal-master-password"), enabled = !busy) { revealed = !revealed }
                    }, update = { password = it })
                    if (authentication is Authentication.Open && state.biometry.status?.state == BiometryState.ENABLED)
                        FlatAction(R.string.biometric_unlock, Modifier.fillMaxWidth().testTag("biometric-unlock"), enabled = !busy) {
                            focus.clearFocus(force = true); keyboard?.hide(); password = ""; revealed = false
                            attempted = true; biometricAttempted = true; state.unlockBiometric(authentication.copy)
                        }
                    if (authentication is Authentication.Open && state.biometry.status?.state == BiometryState.NEEDS_MASTER)
                        Text(stringResource(R.string.biometric_needs_master))
                    if (biometricAttempted && !busy && state.overview == null && state.error)
                        Text(stringResource(R.string.biometric_failed), Modifier.testTag("biometric-open-error"), color = MaterialTheme.colorScheme.error)
                    if (creating) PlainField(repeat, R.string.repeat_master_password, "repeat-master-password", transformation = PasswordVisualTransformation(), keyboard = KeyboardOptions(keyboardType = KeyboardType.Password), update = { repeat = it })
                    if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                    if (state.error || (recoveryAttempted && state.recovery.failure == RecoveryFailure.SOURCE_PASSWORD)) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
                    if (retained != null) FlatAction(R.string.continue_action, Modifier.fillMaxWidth().testTag("continue-recovery"), enabled = !busy) {
                        focus.clearFocus(force = true); keyboard?.hide(); resume(retained)
                    }
                    if (authentication is Authentication.Open && retained == null) FlatAction(R.string.recovery_read_source, Modifier.fillMaxWidth().testTag("read-for-recovery"),
                        enabled = !busy && password.isNotEmpty() && state.recovery.pendingLoaded && !state.recovery.pendingFailure) {
                        focus.clearFocus(force = true); keyboard?.hide(); attempted = true; recoveryAttempted = true
                        state.authenticateRecovery(authentication.copy, password, operation)
                    }
                    if (state.recovery.pendingFailure) {
                        Text(stringResource(R.string.recovery_status_failed), color = MaterialTheme.colorScheme.error)
                        FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("auth-reload-recoveries"), enabled = !busy, action = state.recovery::reload)
                    }
                    if (!creating) FlatAction(R.string.choose_another_database, Modifier.fillMaxWidth().testTag("choose-another-database"), enabled = !busy, action = ::cancel)
                    if (authentication is Authentication.Open) FlatAction(R.string.save_database_copy,
                        Modifier.fillMaxWidth().testTag("save-locked-database-copy"), enabled = !busy) { state.beginDatabaseCopy(authentication.copy) }
                }
            }
        }
}

@Composable
private fun EntryForm(state: DocumentState, attachments: AttachmentActions, copy: (String) -> Unit, copySecret: (String) -> Boolean, historyAction: (String, HistoryRow, HistoryAction) -> Unit) {
    val input = state.entry ?: return
    key(input.source.form.draft) {
        var attribute by remember { mutableStateOf<AttributeRow?>(null) }
        var addAttribute by remember { mutableStateOf(false) }
        if (state.entryTab == 0) {
            EntryFields(input, !state.editingBlocked && state.overview?.writable == true, state.revealed,
                { field, value -> state.edit(field, value) }, state::editTags, state::enableExpiry, state::editExpiry,
                { state.reveal() }, state::hideSecrets, { state.generator = true }, favicon = { state.downloadIcon(input.url, favicon = true) }, display = state.secretDisplay)
        } else if (state.entryTab == 1) {
            input.source.attributes.forEach { row ->
                key(row.id ?: row.name) { EditableAttribute(state, row) { attribute = it } }
            }
            TextButton({ addAttribute = true }, enabled = state.overview?.writable == true) { Text(stringResource(R.string.add_attribute)) }
            if (state.revealed.isNotEmpty()) SecretValue(state.revealed, state::hideSecrets,
                { state.copySecret(copySecret, state.revealedAttribute) }, display = state.secretDisplay)
            Attachments(state, attachments, state.attachments, input.source.entry, editing = state.overview?.writable == true)
        } else if (state.entryTab == 2) {
            AppearanceEditor(state.appearance, state::setIcon,
                { value -> state.setColors(value?.let(ColorEdit::Set) ?: ColorEdit.Clear, ColorEdit.Keep) },
                { value -> state.setColors(ColorEdit.Keep, value?.let(ColorEdit::Set) ?: ColorEdit.Clear) },
                { state.setIcon(null); state.setColors(ColorEdit.Clear, ColorEdit.Clear) }, title = input.title, enabled = !state.editingBlocked, actions = documentIconActions(state, attachments.selectIcon), previewContent = {
                    if (state.appearance?.image != null) DocumentIcon(state, IconScope.Form(input.source.form.draft), state.activeForm?.revision, appearanceIconDrawable(state.appearance))
                    else Glyph(appearanceIconDrawable(state.appearance))
                })
        } else if (state.entryTab == 3) {
            PropertiesContent(state.properties, copy)
        } else {
            HistoryContent(state, input.source.entry, historyAction)
        }
        if (addAttribute) AttributeDialog(null, { addAttribute = false }) { id, name, value, protected ->
            state.editAttribute(id, name, value, protected); addAttribute = false
        }
        attribute?.let { row ->
            var name by remember(row.id) { mutableStateOf(row.name) }
            AlertDialog(onDismissRequest = { attribute = null }, title = { Text(stringResource(R.string.rename)) },
                text = { TextField(name, { name = it }, label = { Text(stringResource(R.string.name)) }) },
                confirmButton = { TextButton({ state.editAttribute(row.id, name, null, row.protected); attribute = null }) { Text(stringResource(R.string.apply)) } },
                dismissButton = { TextButton({ attribute = null }) { Text(stringResource(R.string.cancel)) } })
        }
    }
}

/** Existing values are addressed edits; only constructing/renaming a row has an Apply command. */
@Composable
private fun EditableAttribute(state: DocumentState, row: AttributeRow, rename: (AttributeRow) -> Unit) {
    var value by remember { mutableStateOf(row.value.orEmpty()) }
    var protected by remember { mutableStateOf(row.protected) }
    var typed by remember { mutableStateOf(false) }
    val enabled = !state.editingBlocked && state.overview?.writable == true
    LaunchedEffect(row.value, protected) {
        // Removing protection is an explicit user action; its acknowledged unmasked
        // value can fill untouched input, while later typing retains focus and text.
        if (!typed && !protected && row.value != null) value = row.value.orEmpty()
    }
    EditableAttributeRow(row, value, protected, enabled,
        update = { value = it; typed = true; state.editAttribute(row.id, row.name, it, protected) },
        protect = { protected = it; state.editAttribute(row.id, row.name, null, it) },
        rename = { rename(row.copy(protected = protected)) },
        reveal = { state.reveal(row.id) },
        remove = { state.editAttribute(row.id, row.name, null, protected, true) })
}

@Composable
private fun MetadataForm(state: DocumentState, attachments: AttachmentActions) {
    val input = state.metadata ?: return
    MetadataFields(input, !state.editingBlocked, { state.editMetadata(name = it) }, { state.editMetadata(description = it) }, state::editGroupParent) {
        IconPicker((input.source.icon as? IconView.Lucide)?.key, state::setGroupIcon,
            preview = savedIconDrawable(input.source.icon, R.drawable.ic_folder), enabled = !state.editingBlocked,
            actions = documentIconActions(state, attachments.selectIcon), previewContent = {
                val fallback = savedIconDrawable(input.source.icon, R.drawable.ic_folder)
                if (input.source.icon is IconView.Image) DocumentIcon(state, IconScope.Form(input.source.form.draft), state.activeForm?.revision, fallback) else Glyph(fallback)
            })
    }
    if (input.source.form.kind == FormKind.GROUP || input.source.form.kind == FormKind.DATABASE) {
        FlatAction(R.string.history, Modifier.fillMaxWidth()) { state.showHistory(input.source.form.kind, input.source.form.target) }
    }
    if (input.source.form.kind == FormKind.GROUP && input.source.form.target != null) TextButton({ state.trash(FormKind.GROUP, input.source.form.target!!, null) }) { Text(stringResource(R.string.trash), color = MaterialTheme.colorScheme.error) }
    if (input.source.form.kind == FormKind.DATABASE && state.overview?.managing == true) {
        FlatAction(R.string.protection_title, Modifier.fillMaxWidth().testTag("metadata-protection")) { state.showPolicy() }
        FlatAction(R.string.limits_title, Modifier.fillMaxWidth().testTag("metadata-limits")) { state.showPolicy(PolicyPage.LIMITS) }
    }
}

@Composable
internal fun EntryTabs(selected: Int, select: (Int) -> Unit) {
    val labels = listOf(R.string.details, R.string.additional, R.string.appearance, R.string.properties, R.string.history)
    val captions = labels.map { stringResource(it) }
    val minimums = listOf(92.dp, 160.dp, 72.dp, 124.dp, 126.dp)
    val density = LocalDensity.current
    val measurer = rememberTextMeasurer()
    val style = MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.Medium)
    val widths = captions.mapIndexed { index, caption ->
        maxOf(minimums[index], with(density) { measurer.measure(caption, style, maxLines = 1).size.width.toDp() } + 24.dp)
    }
    val scroll = rememberScrollState()
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val viewport = maxWidth
        LaunchedEffect(selected, widths, viewport) {
            val start = widths.take(selected).fold(8.dp) { sum, width -> sum + width }
            val end = start + widths[selected]
            val preferred = when (selected) { 2 -> 100.dp; 3, 4 -> 216.dp; else -> 0.dp }
            val destination = maxOf(preferred, end - viewport + 8.dp).coerceAtMost(start).coerceAtLeast(0.dp)
            scroll.animateScrollTo(with(density) { destination.roundToPx() })
        }
        Column {
            Row(Modifier.fillMaxWidth().horizontalScroll(scroll).selectableGroup().padding(horizontal = 8.dp)) {
                captions.forEachIndexed { index, caption ->
                    Box(Modifier.width(widths[index]).heightIn(min = 55.dp)
                        .testTag("entry-tab-$index").selectable(selected == index, role = Role.Tab) { select(index) }) {
                        Text(caption, Modifier.align(Alignment.CenterStart).padding(horizontal = 12.dp, vertical = 15.dp),
                            style = style, maxLines = 1, color = if (selected == index) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)
                        if (selected == index) Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(horizontal = 12.dp).height(2.dp).background(MaterialTheme.colorScheme.onSurface))
                    }
                }
            }
            HorizontalDivider()
        }
    }
}
@Composable
internal fun AppearanceSummary(value: AppearanceView?, title: String = "", previewContent: (@Composable () -> Unit)? = null) {
    Text(stringResource(R.string.appearance), style = MaterialTheme.typography.titleMedium)
    NavigationRow(title, appearanceIconDrawable(value), subtitle = stringResource(R.string.entry_preview), leading = previewContent, action = null)
    ReadOnlyField(value?.foreground?.toString(16)?.padStart(8, '0') ?: stringResource(R.string.follow_theme), R.string.foreground_color, "appearance-foreground")
    ReadOnlyField(value?.background?.toString(16)?.padStart(8, '0') ?: stringResource(R.string.follow_theme), R.string.background_color, "appearance-background")
}
@Composable
private fun PropertiesContent(value: EntryProperties?, copy: (String) -> Unit) {
    EntryPropertiesContent(value, copy)
}
@Composable
private fun HistoryContent(state: DocumentState, entry: String?, action: (String, HistoryRow, HistoryAction) -> Unit) {
    EntryHistoryContent(state.entryHistory, state.overview?.writable == true) { row, command -> if (entry != null) action(entry, row, command) }
}
private val documentDateFormatter = java.time.format.DateTimeFormatter.ofPattern("uuuu-MM-dd HH:mm:ss.SSS 'UTC'", java.util.Locale.ROOT)
    .withZone(java.time.ZoneOffset.UTC)

internal fun displayTime(milliseconds: Long): String = documentDateFormatter.format(java.time.Instant.ofEpochMilli(milliseconds))

@Composable
internal fun Field(value: String, label: Int, tag: String, enabled: Boolean, single: Boolean = true, update: (String) -> Unit) {
    PlainField(value, label, tag, enabled = enabled, singleLine = single, structured = true, update = update)
}
@Composable
internal fun SecretValue(value: String, hide: () -> Unit, copy: (String) -> Unit, display: SecretDisplayTicket? = null) {
    val view = androidx.compose.ui.platform.LocalView.current
    val context = androidx.compose.ui.platform.LocalContext.current
    val app = context.applicationContext as TaypeerApplication
    val latestHide by rememberUpdatedState(hide)
    DisposableEffect(view) {
        val observer = view.viewTreeObserver
        val listener = ViewTreeObserver.OnWindowFocusChangeListener { focused ->
            try { app.clipboard.focusChanged(focused) } catch (_: Exception) { /* Copy reports failure separately. */ }
            if (!focused) latestHide()
        }
        observer.addOnWindowFocusChangeListener(listener)
        try { app.clipboard.focusChanged(view.hasWindowFocus()) } catch (_: Exception) { /* Remain unavailable. */ }
        onDispose {
            if (observer.isAlive) observer.removeOnWindowFocusChangeListener(listener)
            try { app.clipboard.focusChanged(context.activity()?.hasWindowFocus() == true) } catch (_: Exception) { /* Remain unavailable. */ }
        }
    }
    Text(value, Modifier.testTag("revealed-secret").secretDisplay(display))
    Row { TextButton(hide) { Text(stringResource(R.string.hide)) }; TextButton({ copy(value) }, Modifier.testTag("copy-revealed-secret")) { Text(stringResource(R.string.copy)) } }
}
private fun Context.activity(): android.app.Activity? = when (this) {
    is android.app.Activity -> this
    is ContextWrapper -> baseContext.activity()
    else -> null
}
@Composable
private fun AttributeDialog(row: AttributeRow?, dismiss: () -> Unit, confirm: (String?, String, String?, Boolean) -> Unit) {
    var name by remember { mutableStateOf(row?.name.orEmpty()) }
    var value by remember { mutableStateOf(row?.value.orEmpty()) }
    var changed by remember { mutableStateOf(row == null || row.protected.not()) }
    var protected by remember { mutableStateOf(row?.protected ?: true) }
    AlertDialog(onDismissRequest = dismiss, title = { Text(stringResource(R.string.attribute)) }, text = {
        Column {
            OutlinedTextField(name, { name = it }, label = { Text(stringResource(R.string.name)) }, modifier = Modifier.testTag("new-attribute-name"))
            OutlinedTextField(value, { value = it; changed = true }, label = { Text(stringResource(R.string.value)) }, visualTransformation = PasswordVisualTransformation(), modifier = Modifier.testTag("new-attribute-value"))
            Row(verticalAlignment = Alignment.CenterVertically) { Text(stringResource(R.string.protected_value), Modifier.weight(1f)); Switch(protected, { protected = it }) }
        }
    }, confirmButton = { TextButton({ confirm(row?.id, name, if (changed) value else null, protected) }) { Text(stringResource(R.string.apply)) } }, dismissButton = { TextButton(dismiss) { Text(stringResource(R.string.cancel)) } })
}
@Composable
internal fun InputDialog(title: Int, label: Int, dismiss: () -> Unit, secret: Boolean = false, confirm: (String) -> Unit) {
    var value by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = dismiss, title = { Text(stringResource(title)) }, text = {
        Column { OutlinedTextField(value, { value = it }, label = { Text(stringResource(label)) }, visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None,
            keyboardOptions = KeyboardOptions(keyboardType = if (secret) KeyboardType.Password else KeyboardType.Text))
            if (secret) Text(stringResource(R.string.rotation_warning))
        }
    },
        confirmButton = { TextButton({ confirm(value) }) { Text(stringResource(R.string.continue_action)) } }, dismissButton = { TextButton(dismiss) { Text(stringResource(R.string.cancel)) } })
}

private fun saveText(label: SaveLabel) = when (label) {
    SaveLabel.CLEAN -> R.string.save_clean
    SaveLabel.DIRTY -> R.string.save_dirty
    SaveLabel.SAVING -> R.string.save_running
    SaveLabel.SAVED -> R.string.save_saved
    SaveLabel.FAILED -> R.string.save_failed
}
private fun formText(kind: FormKind) = when (kind) {
    FormKind.ENTRY -> R.string.entry
    FormKind.NEW_ENTRY -> R.string.new_entry
    FormKind.GROUP -> R.string.group
    FormKind.NEW_GROUP -> R.string.new_group
    FormKind.DATABASE -> R.string.database_info
}
