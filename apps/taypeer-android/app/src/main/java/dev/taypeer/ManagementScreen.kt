package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

internal data class ManagementRecipient(val id: String, val label: String)

@Composable
internal fun ManagementScreen(state: ManagementState, databaseName: String?, localName: String?, managing: Boolean) {
    when (val route = state.route) {
        is ManagementRoute.Transfer -> {
            val row = state.row(route.operation)
            val peers = state.peers.map { ManagementRecipient(it.id, it.id.take(12)) }
            ManagementTransferForm(databaseName, localName?.takeIf { managing }, peers, route.recipient,
                state.busy(route.operation), state.frozen(route.operation), row, state.problem(route.operation),
                state.observation?.takeIf { it.intent.operation == route.operation }, state.observing, state.observationFailed,
                state::choose, state::submit, state::close, { row?.let(state::inspectPeer) })
        }
        is ManagementRoute.Decision -> {
            val row = state.row(route.intent.operation) ?: return
            ManagementDecisionForm(databaseName, route.intent.manager.take(12), localName,
                row, state.busy(route.intent.operation), state.decisionPending(route.intent.operation), state.problem(route.intent.operation),
                { state.decide(true) }, { state.decide(false) }, { state.retry(row) }, state::close)
        }
        null -> Unit
    }
}

/** Saved transfer-control form. Selecting a peer is not a network request or a readiness claim. */
@Composable
internal fun ManagementTransferForm(databaseName: String?, managerLabel: String?, recipients: List<ManagementRecipient>,
    selected: String?, busy: Boolean, frozen: Boolean, row: ManagementHandoffView?, problem: ManagementProblem?,
    observation: ManagementPeerStatusView?, observing: Boolean, observationFailed: Boolean,
    choose: (String) -> Unit, submit: () -> Unit, close: () -> Unit, inspect: () -> Unit) {
    BackHandler(onBack = close)
    val recipient = recipients.firstOrNull { it.id == selected }
    val label = recipient?.label ?: selected?.take(12)
    Scaffold(Modifier.testTag("transfer-management-screen"),
        topBar = { PhoneBar(stringResource(R.string.transfer_management), back = close) },
        bottomBar = {
            FormActions(close, submit, enabled = !busy && selected != null && row?.phase?.let(::terminal) != true,
                tag = "confirm-management-transfer", label = if (frozen) R.string.continue_action else R.string.transfer_management_action,
                cancelLabel = if (frozen) R.string.close else R.string.cancel, cancelEnabled = true)
        }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(if (managerLabel == null) databaseName ?: stringResource(R.string.working_copy)
                else stringResource(R.string.management_current_manager, databaseName ?: stringResource(R.string.working_copy), managerLabel),
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            recipients.forEach { value ->
                NavigationRow(value.label, R.drawable.ic_monitor, Modifier.testTag("management-recipient-${value.id}")
                    .background(if (value.id == selected) MaterialTheme.colorScheme.surfaceVariant else MaterialTheme.colorScheme.surface,
                        RoundedCornerShape(6.dp))
                    .semantics { this.selected = value.id == selected; if (busy || frozen) disabled() },
                    stringResource(R.string.management_readiness_unknown),
                    action = if (busy || frozen) null else ({ choose(value.id) }))
            }
            if (recipients.isEmpty()) Text(stringResource(R.string.management_no_recipient), color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (label != null) Text(stringResource(R.string.management_transfer_hint, label), color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (row != null) {
                ManagementPhaseText(row)
                FlatAction(R.string.management_check_peer, Modifier.fillMaxWidth().testTag("check-management-peer"),
                    enabled = !busy && !observing, action = inspect)
            }
            observation?.let { ManagementPeerObservation(it) }
            if (observationFailed) Text(stringResource(R.string.management_peer_check_failed), color = MaterialTheme.colorScheme.error,
                modifier = Modifier.testTag("management-peer-error"))
            if (busy || observing) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("management-busy"))
            problem?.let { ManagementProblemText(it) }
        }
    }
}

/** Cancel hides the prompt; only the explicitly labelled Reject records a refusal. */
@Composable
internal fun ManagementDecisionForm(databaseName: String?, managerLabel: String, localName: String?, row: ManagementHandoffView,
    busy: Boolean, pending: Boolean, problem: ManagementProblem?, accept: () -> Unit, reject: () -> Unit,
    retry: () -> Unit, close: () -> Unit) {
    BackHandler(onBack = close)
    val untouched = row.phase == ManagementPhase.WAITING_DECISION && !pending
    Scaffold(Modifier.testTag("accept-management-screen"),
        topBar = { PhoneBar(stringResource(R.string.accept_management_title), back = close) },
        bottomBar = {
            FormActions(close, if (untouched) accept else retry, enabled = !busy && !terminal(row.phase),
                tag = "confirm-management-accept", label = if (untouched) R.string.accept_management_action else R.string.continue_action,
                primary = untouched, cancelLabel = if (untouched) R.string.cancel else R.string.close, cancelEnabled = true)
        }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(if (localName == null) stringResource(R.string.management_accept_hint, managerLabel, databaseName ?: stringResource(R.string.working_copy))
                else stringResource(R.string.management_accept_named_hint, managerLabel, databaseName ?: stringResource(R.string.working_copy), localName))
            if (untouched) FlatAction(R.string.reject, Modifier.fillMaxWidth().testTag("reject-management"), ghost = true, action = reject)
            else ManagementPhaseText(row)
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("management-busy"))
            problem?.let { ManagementProblemText(it) }
        }
    }
}

@Composable
internal fun DeviceActionSheet(label: String, canManage: Boolean, sync: () -> Unit, transfer: () -> Unit,
    revoke: () -> Unit, close: () -> Unit) {
    BackHandler(onBack = close)
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = .55f)).clickable(onClick = close)
        .testTag("device-actions-sheet")) {
        Surface(Modifier.align(Alignment.BottomCenter).fillMaxWidth().clickable {},
            shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp)) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Box(Modifier.fillMaxWidth().height(20.dp)) {
                    Box(Modifier.align(Alignment.TopCenter).padding(top = 8.dp).size(40.dp, 4.dp)
                        .background(MaterialTheme.colorScheme.onSurfaceVariant, RoundedCornerShape(2.dp)))
                }
                DeviceActionContent(label, canManage, sync, transfer, revoke)
            }
        }
    }
}

@Composable
internal fun DeviceActionContent(label: String, canManage: Boolean, sync: () -> Unit, transfer: () -> Unit, revoke: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, Modifier.heightIn(min = 28.dp), style = MaterialTheme.typography.titleLarge)
        DeviceTextAction(R.string.sync, "sync-device", true, sync)
        DeviceTextAction(R.string.transfer_management, "transfer-management", canManage, transfer)
        DeviceTextAction(R.string.revoke, "revoke-device", canManage, revoke, destructive = true)
    }
}

@Composable
private fun DeviceTextAction(label: Int, tag: String, enabled: Boolean, action: () -> Unit, destructive: Boolean = false) {
    Text(stringResource(label), Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable(enabled = enabled, role = Role.Button, onClick = action)
        .padding(vertical = 16.dp).testTag(tag), color = when {
        !enabled -> MaterialTheme.colorScheme.onSurface.copy(alpha = .38f)
        destructive -> MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.onSurface
    })
}
