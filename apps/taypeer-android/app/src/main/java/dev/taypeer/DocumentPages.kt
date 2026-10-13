package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*
import dev.taypeer.platform.ExchangeScheduler

@Composable
internal fun HistoryScreen(state: DocumentState, attachments: AttachmentActions, selection: HistorySelection, copy: (String) -> Unit,
    copySecret: (String) -> Boolean = { false },
    launch: HistoryLaunch? = null, consumed: () -> Unit = {}) {
    var reviewed by remember(selection.kind, selection.target) { mutableStateOf(setOf<String>()) }
    var viewing by remember { mutableStateOf(state.historicalRevision != null) }
    var restoring by remember { mutableStateOf<HistoryRow?>(null) }
    var purging by remember { mutableStateOf<ReviewedPurge?>(null) }
    var busy by remember { mutableStateOf(false) }
    val row = selection.rows.firstOrNull { it.id == state.historicalRevision }
    val writable = state.overview?.writable == true
    LaunchedEffect(state.historicalRevision) { viewing = row != null && selection.kind == FormKind.ENTRY }
    fun inspect(version: HistoryRow) { state.clearError(); viewing = selection.kind == FormKind.ENTRY; state.selectRevision(version) }
    fun restore(version: HistoryRow) {
        if (state.historicalRevision != version.id) state.selectRevision(version)
        state.hideSecrets(); state.clearError(); restoring = version
    }
    fun revisionAction(version: HistoryRow, action: HistoryAction) {
        if (selection.kind != FormKind.ENTRY && action != HistoryAction.PURGE) {
            val mode = when (action) {
                HistoryAction.VIEW -> MetadataHistoryMode.SAVED
                HistoryAction.COMPARE -> MetadataHistoryMode.COMPARE
                HistoryAction.RESTORE -> MetadataHistoryMode.RESTORE
                HistoryAction.PURGE -> return
            }
            state.hideSecrets(); state.clearError()
            state.metadataHistory.open(selection.kind, selection.target, version.id, mode)
            return
        }
        when (action) {
            HistoryAction.VIEW -> inspect(version)
            HistoryAction.COMPARE -> {
                inspect(version)
                selection.target?.let { state.comparison.open(it, version.id) }
            }
            HistoryAction.RESTORE -> restore(version)
            HistoryAction.PURGE -> {
                state.hideSecrets(); state.clearError(); viewing = false
                purging = ReviewedPurge(selection, listOf(version.id), newOperation())
            }
        }
    }
    LaunchedEffect(launch) {
        val request = launch ?: return@LaunchedEffect
        if (request.selection === selection) {
            selection.rows.firstOrNull { it.id == request.revision }?.let { revisionAction(it, request.action) }
            consumed()
        }
    }
    restoring?.let { version ->
        val entry = selection.target ?: return
        val request = state.restorations.request(entry, version.id)
        RestoreVersionScreen(version, state.overview?.groups.orEmpty(), state.restorations.busy, state.error || state.restorations.error,
            request?.destination, request != null, confirm = { destination ->
                if (!state.restorations.busy) {
                    state.clearError()
                    val completed: (Boolean) -> Unit = { confirmed -> if (confirmed) restoring = null }
                    if (request != null) state.restorations.retry(request, completed)
                    else state.restoreRevision(entry, version.id, newOperation(), destination, completed)
                }
            }, close = { if (!state.restorations.busy) { restoring = null; state.clearError() } },
            initialDestination = state.historicalEntry?.takeIf { state.historicalRevision == version.id }?.group
                ?.takeIf { group -> state.overview?.groups?.any { it.id == group } == true })
        return
    }
    val comparison = state.comparison
    if (comparison.showing && row != null) {
        HistoryComparisonScreen(comparison.saved, comparison.current, row, comparison.savedPassword, comparison.currentPassword,
            comparison.busy, comparison.error, writable, comparison::reveal, { comparison.copyPassword(it, copySecret) }, comparison::retry,
            comparison::close, { restore(row) }, comparison.display(ComparisonSide.SAVED), comparison.display(ComparisonSide.CURRENT))
        return
    }
    if (viewing && row != null && selection.kind == FormKind.ENTRY) {
        SavedRevisionScreen(row, state.historicalEntry, if (state.revealedAttribute == null) state.revealed.ifEmpty { null } else null,
            state.error, writable, { if (state.revealed.isEmpty()) state.reveal() else state.hideSecrets() }, copy, { state.copySecret(copySecret) },
            compare = { selection.target?.let { state.hideSecrets(); comparison.open(it, row.id) } }, restore = { restore(row) },
            close = { state.hideSecrets(); viewing = false }, additional = {
                SavedAttributesContent(state.historicalAttributes, state.revealedAttribute, state.revealed,
                    state::reveal, { state.copySecret(copySecret, it) }, copy, display = { state.secretDisplay })
                state.historicalEntry?.let { Attachments(state, attachments, state.historicalAttachments, it.id, row.id) }
            }, properties = state.historicalProperties, selectedTab = state.historicalTab,
            selectTab = state::selectHistoricalTab, revisions = selection.rows, revisionAction = ::revisionAction, iconPreview = {
                val appearance = state.historicalProperties?.appearance
                if (appearance?.image != null && selection.target != null) DocumentIcon(state, IconScope.Revision(selection.target, row.id), appearance.image, appearanceIconDrawable(appearance))
                else Glyph(appearanceIconDrawable(appearance))
            }, display = state.secretDisplay)
        return
    }
    BackHandler { if (!busy) state.dismissHistory() }
    Scaffold(modifier = Modifier.testTag("history-list-screen"), topBar = { PhoneBar(stringResource(R.string.history), back = { if (!busy) state.dismissHistory() }) }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
            HistoryListContent(selection.rows, reviewed, writable, true,
                ::inspect, { id, checked -> reviewed = if (checked) reviewed + id else reviewed - id },
                purge = { state.clearError(); purging = ReviewedPurge(selection, reviewed.toList(), newOperation()) })
        }
    }
    purging?.let { request -> AlertDialog(onDismissRequest = { if (!busy) purging = null }, title = { Text(stringResource(R.string.purge_history)) },
        text = { Column { Text(stringResource(R.string.purge_history_warning)); if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error) } },
        confirmButton = { TextButton({ if (!busy) { busy = true; state.purgeHistory(request.selection, request.revisions, request.operation) { confirmed -> busy = false; if (confirmed) { purging = null; reviewed = emptySet() } } } }, enabled = !busy, modifier = Modifier.testTag("confirm-purge-history")) { Text(stringResource(R.string.delete_permanently)) } },
        dismissButton = { TextButton({ purging = null }, enabled = !busy) { Text(stringResource(R.string.cancel)) } }) }
}

private data class ReviewedPurge(val selection: HistorySelection, val revisions: List<String>, val operation: String)

/** Created only after guarded navigation completes; identity prevents a later route replay. */
internal data class HistoryLaunch(val selection: HistorySelection, val revision: String, val action: HistoryAction)

@Composable
internal fun ExchangeScreen(state: DocumentState, exchange: ExchangeView, copy: (String) -> Unit) {
    var deviceMenu by remember(exchange.database) { mutableStateOf<DeviceRow?>(null) }
    var approving by remember { mutableStateOf<String?>(null) }
    var rotating by remember { mutableStateOf<String?>(null) }
    var rotation by remember { mutableStateOf(false) }
    var showingInvitation by remember { mutableStateOf(false) }
    val context = androidx.compose.ui.platform.LocalContext.current
    LaunchedEffect(state.invitation) { if (state.invitation.isNotEmpty()) showingInvitation = true }
    if (showingInvitation) {
        InvitationScreen(state.invitation, state.overview?.name, { state.hideSecrets(); showingInvitation = false }, copy, state::invite)
        return
    }
    if (rotation) {
        PasswordRotationScreen(state, rotating) { rotation = false; rotating = null }
        return
    }
    BackHandler { state.dismissExchange() }
    Scaffold(topBar = { PhoneBar(stringResource(R.string.devices_exchange), back = state::dismissExchange) {
        IconAction(R.drawable.ic_refresh_cw, R.string.refresh, action = state::exchange)
    } }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            ExchangeStatus(exchange, state.applicationStatus)
            ManagementHandoffs(state.management, state::reviewManagement)
            TextButton({ ExchangeScheduler.stopForeground(context) }, enabled = exchange.running) { Text(stringResource(R.string.stop_exchange)) }
            exchange.devices.forEach { device ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        val name = (context.applicationContext as TaypeerApplication).deviceSettings.settings?.deviceName
                        ExchangeDeviceLabel(device, name)
                        DeviceExchangeStatus(device)
                    }
                    if (if (device.local) state.overview?.managing == true else device.manager) Text(stringResource(R.string.managing_device))
                    if (!device.local) IconAction(R.drawable.ic_ellipsis, R.string.actions,
                        Modifier.testTag("device-menu-${device.id}")) { deviceMenu = device }
                }
            }
            exchange.invitations.filter { it.phase == InvitationPhase.REQUESTED }.forEach { request ->
                Text(request.recipient?.take(12).orEmpty())
                Row { TextButton({ approving = request.request }) { Text(stringResource(R.string.approve)) }; TextButton({ state.reject(request.request) }) { Text(stringResource(R.string.reject)) } }
            }
            if (state.overview?.managing == true) {
                FlatAction(R.string.share_database, Modifier.fillMaxWidth(), action = state::invite)
                FlatAction(R.string.change_password, Modifier.fillMaxWidth().testTag("open-change-password")) { state.clearError(); rotating = null; rotation = true }
            }
        }
    }
    deviceMenu?.let { device -> DeviceActionSheet(device.id.take(12), state.overview?.managing == true,
        sync = { deviceMenu = null; state.exchange() },
        transfer = { deviceMenu = null; state.beginManagementTransfer(device.id) },
        revoke = { deviceMenu = null; state.clearError(); rotating = device.id; rotation = true }, close = { deviceMenu = null }) }
    approving?.let { request -> AlertDialog(onDismissRequest = { approving = null }, title = { Text(stringResource(R.string.approve_device)) }, text = { Text(stringResource(R.string.approve_device_warning)) },
        confirmButton = { TextButton({ state.approve(request); approving = null }) { Text(stringResource(R.string.approve)) } }, dismissButton = { TextButton({ approving = null }) { Text(stringResource(R.string.cancel)) } }) }
}
@Composable
internal fun ExchangeDeviceLabel(device: DeviceRow, appliedName: String?) {
    Text(if (device.local) appliedName ?: device.id.take(12) else device.id.take(12))
}
