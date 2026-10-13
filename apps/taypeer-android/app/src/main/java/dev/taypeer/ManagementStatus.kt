package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

@Composable
internal fun ManagementPhaseText(row: ManagementHandoffView) {
    val label = when (row.phase) {
        ManagementPhase.PROPOSED -> R.string.management_proposed
        ManagementPhase.WAITING_DECISION -> R.string.management_waiting_decision
        ManagementPhase.ACCEPT_REQUESTED -> R.string.management_accept_requested
        ManagementPhase.CONSENT_RECORDED -> R.string.management_consent_recorded
        ManagementPhase.REJECTED -> R.string.management_rejected
        ManagementPhase.SOURCE_COMMITTED -> R.string.management_source_committed
        ManagementPhase.AWAITING_APPLY -> R.string.management_awaiting_apply
        ManagementPhase.COMPLETED -> if (row.applied != null) R.string.management_completed else R.string.management_status_failed
    }
    Text(stringResource(label), Modifier.testTag("management-phase-${row.intent.operation}"), color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
internal fun ManagementProblemText(problem: ManagementProblem) {
    Text(stringResource(when (problem) {
        ManagementProblem.UNCONFIRMED -> R.string.management_unconfirmed
        ManagementProblem.NEEDS_UNLOCK -> R.string.management_needs_unlock
        ManagementProblem.PENDING_APPLY -> R.string.management_pending_apply
        ManagementProblem.STALE -> R.string.management_stale
        ManagementProblem.WRONG_ROLE -> R.string.management_wrong_role
        ManagementProblem.UNKNOWN -> R.string.management_unknown
        ManagementProblem.CONFLICT -> R.string.management_conflict
        ManagementProblem.CAPACITY -> R.string.management_capacity
        ManagementProblem.UNSUPPORTED -> R.string.management_unsupported
    }), Modifier.testTag("management-error"), color = MaterialTheme.colorScheme.error)
}

@Composable
internal fun ManagementPeerObservation(value: ManagementPeerStatusView) {
    Text(stringResource(when (value.readiness) {
        ManagementReadinessView.Locked -> R.string.management_last_check_locked
        ManagementReadinessView.PendingApply -> R.string.management_last_check_pending
        is ManagementReadinessView.Ready -> R.string.management_last_check_ready
    }), Modifier.testTag("management-peer-observation"), style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** Protected local phases are shown while locked; only explicit Retry/Check performs networking. */
@Composable
internal fun ManagementHandoffs(state: ManagementState, review: (ManagementHandoffView) -> Unit) {
    if (state.preparing) LinearProgressIndicator(Modifier.fillMaxWidth())
    if (state.loadFailed) {
        Text(stringResource(R.string.management_status_failed), Modifier.padding(16.dp).testTag("management-status-error"),
            color = MaterialTheme.colorScheme.error)
        FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("reload-management"), action = state::load)
        return
    }
    state.rows.forEach { row ->
        Column(Modifier.fillMaxWidth().padding(16.dp).testTag("management-handoff-${row.intent.operation}"),
            verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.transfer_management), style = MaterialTheme.typography.titleMedium)
            Text((if (row.intent.manager == state.localDevice) row.intent.recipient else row.intent.manager).take(12))
            ManagementPhaseText(row)
            state.problem(row.intent.operation)?.let { ManagementProblemText(it) }
            if (row.intent.recipient == state.localDevice && row.phase == ManagementPhase.WAITING_DECISION && !state.decisionPending(row.intent.operation))
                FlatAction(R.string.management_review, Modifier.fillMaxWidth().testTag("review-management-${row.intent.operation}"),
                    enabled = state.canDecide && !state.busy(row.intent.operation)) { review(row) }
            else if (!terminal(row.phase)) FlatAction(R.string.continue_action,
                Modifier.fillMaxWidth().testTag("retry-management-${row.intent.operation}"), enabled = !state.busy(row.intent.operation)) { state.retry(row) }
            if (state.busy(row.intent.operation)) LinearProgressIndicator(Modifier.fillMaxWidth())
        }
    }
    state.unconfirmedProposals().forEach { (operation, recipient) ->
        Column(Modifier.fillMaxWidth().padding(16.dp).testTag("unconfirmed-management-$operation"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.transfer_management), style = MaterialTheme.typography.titleMedium)
            Text(recipient.take(12))
            ManagementProblemText(state.problem(operation) ?: ManagementProblem.UNCONFIRMED)
            FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-management-$operation"), enabled = !state.busy(operation)) { state.retryUnconfirmed(operation) }
        }
    }
}
