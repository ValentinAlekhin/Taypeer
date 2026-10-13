package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

@Composable
internal fun RecoveryScreen(state: RecoveryState, deviceName: String?, restart: (RecoveryTarget) -> Unit) {
    RecoveryForm(state.name, state.password, state.repeated, state.busy, state.frozen, state.failure,
        deviceName, state::updateName, state::updatePassword, state::updateRepeated, state::submit, state::close,
        fresh = { state.newOperationRequested()?.let(restart) })
}

/** The saved FIG form, also used by public visual fixtures. Rust validates exact input. */
@Composable
internal fun RecoveryForm(name: String, password: String, repeated: String, busy: Boolean, frozen: Boolean,
    failure: RecoveryFailure?, deviceName: String?, updateName: (String) -> Unit, updatePassword: (String) -> Unit,
    updateRepeated: (String) -> Unit, submit: () -> Unit, close: () -> Unit, fresh: () -> Unit) {
    var reveal by remember(LocalSecretMaskEpoch.current) { mutableStateOf(false) }
    var revealRepeat by remember(LocalSecretMaskEpoch.current) { mutableStateOf(false) }
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val editable = !busy && !frozen
    val matching = password == repeated
    fun cancel() { if (editable) { focus.clearFocus(force = true); keyboard?.hide(); close() } }
    BackHandler { cancel() }
    LaunchedEffect(busy, frozen) { if (busy || frozen) { reveal = false; revealRepeat = false } }
    Scaffold(Modifier.testTag("recovery-screen"),
        topBar = { PhoneBar(stringResource(R.string.recover_management), back = ::cancel) },
        bottomBar = {
            FormActions(::cancel, {
                focus.clearFocus(force = true); keyboard?.hide(); submit()
            }, enabled = !busy && failure != RecoveryFailure.NEW_OPERATION && password.isNotEmpty() && matching,
                tag = "confirm-recovery", label = if (frozen) R.string.retry else R.string.create,
                cancelEnabled = editable, primary = true)
        }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(if (deviceName == null) stringResource(R.string.recovery_intro)
                else stringResource(R.string.recovery_intro_named, deviceName), color = MaterialTheme.colorScheme.onSurfaceVariant)
            PlainField(name, R.string.recovery_name, "recovery-name", enabled = editable, update = updateName)
            PlainField(password, R.string.new_master_password, "recovery-password", enabled = editable,
                transformation = if (reveal && editable) VisualTransformation.None else PasswordVisualTransformation(),
                keyboard = KeyboardOptions(keyboardType = KeyboardType.Password), actions = {
                    IconAction(if (reveal) R.drawable.ic_eye_off else R.drawable.ic_eye, if (reveal) R.string.hide else R.string.reveal,
                        Modifier.testTag("reveal-recovery-password"), enabled = editable) { reveal = !reveal }
                }, update = updatePassword)
            PlainField(repeated, R.string.repeat_master_password, "recovery-repeat", enabled = editable,
                transformation = if (revealRepeat && editable) VisualTransformation.None else PasswordVisualTransformation(),
                keyboard = KeyboardOptions(keyboardType = KeyboardType.Password), actions = {
                    IconAction(if (revealRepeat) R.drawable.ic_eye_off else R.drawable.ic_eye, if (revealRepeat) R.string.hide else R.string.reveal,
                        Modifier.testTag("reveal-recovery-repeat"), enabled = editable) { revealRepeat = !revealRepeat }
                }, update = updateRepeated)
            if (repeated.isNotEmpty() && !matching) Text(stringResource(R.string.rotation_password_mismatch), color = MaterialTheme.colorScheme.error)
            Text(stringResource(R.string.recovery_warning), color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("recovery-busy"))
            failure?.let { reason ->
                Text(stringResource(when (reason) {
                    RecoveryFailure.INPUT -> R.string.invalid_form
                    RecoveryFailure.UNCONFIRMED -> R.string.recovery_unconfirmed
                    RecoveryFailure.NEW_OPERATION -> R.string.recovery_new_operation_required
                    RecoveryFailure.SOURCE_PASSWORD -> R.string.operation_failed
                }), Modifier.testTag("recovery-error"), color = MaterialTheme.colorScheme.error)
                if (reason == RecoveryFailure.NEW_OPERATION) FlatAction(R.string.recovery_start_new,
                    Modifier.fillMaxWidth().testTag("recovery-new-operation"), enabled = !busy, action = fresh)
            }
        }
    }
}

@Composable
internal fun PendingRecoveries(state: RecoveryState, resume: (PendingRecoveryView) -> Unit) {
    if (state.pendingFailure) {
        Text(stringResource(R.string.recovery_status_failed), color = MaterialTheme.colorScheme.error,
            modifier = Modifier.padding(16.dp).testTag("pending-recovery-error"))
        FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("reload-recoveries"), action = state::reload)
    } else state.pending.forEach { row ->
        Column(Modifier.fillMaxWidth().padding(16.dp).testTag("pending-recovery-${row.operation}"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.recovery_pending), style = MaterialTheme.typography.titleMedium)
            Text(row.source.database.take(12), color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall)
            Text(stringResource(when (row.phase) {
                RecoveryPhase.SOURCE_FROZEN -> R.string.recovery_source_frozen
                RecoveryPhase.CANDIDATE_PUBLISHED -> R.string.recovery_candidate_published
                RecoveryPhase.PROOF_CONFIRMED -> R.string.recovery_proof_confirmed
                RecoveryPhase.ADOPTED -> R.string.recovery_adopted
                RecoveryPhase.REFUSED_BEFORE_PUBLICATION -> R.string.recovery_new_operation_required
            }), style = MaterialTheme.typography.bodySmall)
            FlatAction(R.string.continue_action, Modifier.fillMaxWidth().testTag("resume-recovery-${row.operation}"), action = { resume(row) })
        }
    }
}
