package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.CalibrationView
import dev.taypeer.bridge.PolicyDetailsView
import java.util.Locale

@Composable
internal fun PolicySettingsScreen(state: PolicyState, databaseName: String?) {
    val page = state.page ?: return
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    fun close() { if (state.closeEnabled) { focus.clearFocus(force = true); keyboard?.hide(); state.close() } }
    BackHandler { close() }
    Scaffold(modifier = Modifier.testTag("policy-settings-screen"),
        topBar = { PhoneBar(stringResource(if (page == PolicyPage.PROTECTION) R.string.protection_title else R.string.limits_title), back = ::close) },
        bottomBar = { FormActions(::close, {
            focus.clearFocus(force = true); keyboard?.hide(); state.apply()
        }, enabled = !state.busy && !state.calibrating && (state.retained != null || (state.details != null && state.managing)),
            tag = "apply-policy", label = if (state.retained == null) R.string.apply else R.string.retry,
            cancelEnabled = state.closeEnabled) }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("policy-busy"))
            state.details?.let { details ->
                if (page == PolicyPage.PROTECTION) {
                    ProtectionContent(details, state.target ?: details.policy.kdfTargetMs, state.preview,
                        state.calibrating, state.editable, databaseName, state.managing, state::selectTarget, state::calibrate)
                    if (state.target != details.policy.kdfTargetMs) {
                        PlainField(state.password, R.string.master_password, "policy-password", enabled = state.editable,
                            keyboard = KeyboardOptions(keyboardType = KeyboardType.Password), transformation = PasswordVisualTransformation(), update = state::editPassword)
                        Text(stringResource(R.string.protection_password_needed), style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                } else LimitsContent(details, state.attachment, state.total, state.editable, state::editAttachment, state::editTotal)
                if (!state.managing) Text(stringResource(R.string.policy_readonly), color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (state.details == null) state.retained?.let { held ->
                val locale = LocalConfiguration.current.locales[0] ?: Locale.getDefault()
                Text(stringResource(R.string.policy_retained_summary,
                    PolicyUnits.mib(held.value.attachmentBytes, locale), PolicyUnits.mib(held.value.totalAttachmentBytes, locale),
                    PolicyUnits.seconds(held.value.kdfTargetMs.toULong(), locale)), Modifier.testTag("policy-retained-intent"))
            }
            state.failure?.let { failure ->
                Text(stringResource(when (failure) {
                    PolicyFailure.READ -> R.string.policy_read_failed
                    PolicyFailure.INPUT -> R.string.policy_number_invalid
                    PolicyFailure.REJECTED -> R.string.policy_rejected
                    PolicyFailure.UNCERTAIN -> R.string.policy_retry_exact
                    PolicyFailure.CALIBRATION -> R.string.policy_calibration_failed
                }), Modifier.testTag("policy-error"), color = MaterialTheme.colorScheme.error)
                if (failure == PolicyFailure.READ) FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("reload-policy"),
                    enabled = !state.busy, action = state::reload)
            }
        }
    }
}

/** Same product builder used by the FIG fixtures, with actual numeric views. */
@Composable
internal fun ProtectionContent(details: PolicyDetailsView, target: UInt, preview: CalibrationView?,
    calibrating: Boolean, editable: Boolean, databaseName: String?, managing: Boolean, selectTarget: (UInt) -> Unit, calibrate: () -> Unit) {
    val locale = LocalConfiguration.current.locales[0] ?: Locale.getDefault()
    databaseName?.let { Text(stringResource(if (managing) R.string.policy_database_manager else R.string.policy_database_readonly, it)) }
    SelectorField(R.string.protection_target, stringResource(R.string.policy_seconds, PolicyUnits.seconds(target.toULong(), locale)),
        "policy-target", enabled = editable) { dismiss ->
        ((500u..5000u step 500).toList() + target).distinct().sorted().forEach { milliseconds ->
            DropdownMenuItem({ Text(stringResource(R.string.policy_seconds, PolicyUnits.seconds(milliseconds.toULong(), locale))) },
                { dismiss(); selectTarget(milliseconds) }, Modifier.testTag("policy-target-$milliseconds"))
        }
    }
    SelectorField(R.string.protection_memory, stringResource(R.string.policy_mib, PolicyUnits.mib(details.parameters.memoryKib.toULong() * 1024uL, locale)),
        "policy-memory", enabled = false) { }
    Text(stringResource(R.string.protection_algorithm_range), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Text(stringResource(R.string.protection_passes, details.parameters.iterations.toString(), details.parameters.parallelism.toString()),
        Modifier.testTag("policy-current-parameters"), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Text(details.passwordDerivationMs?.let { stringResource(R.string.protection_actual_time, PolicyUnits.seconds(it, locale)) }
        ?: stringResource(R.string.protection_time_unavailable), Modifier.testTag("policy-actual-time"), style = MaterialTheme.typography.bodySmall)
    FlatAction(R.string.protection_calibrate, Modifier.fillMaxWidth().testTag("calibrate-policy"), enabled = editable && !calibrating, action = calibrate)
    if (calibrating) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("policy-calibrating"))
    preview?.let { value ->
        Text(stringResource(R.string.protection_preview, PolicyUnits.seconds(value.targetMs.toULong(), locale),
            value.parameters.iterations.toString(), PolicyUnits.seconds(value.elapsedMs, locale)),
            Modifier.testTag("policy-calibration-preview"), style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
internal fun LimitsContent(details: PolicyDetailsView, attachment: String, total: String, editable: Boolean,
    editAttachment: (String) -> Unit, editTotal: (String) -> Unit) {
    val locale = LocalConfiguration.current.locales[0] ?: Locale.getDefault()
    PlainField(attachment, R.string.limits_attachment_mib, "policy-attachment", enabled = editable,
        keyboard = KeyboardOptions(keyboardType = KeyboardType.Decimal), update = editAttachment)
    PlainField(total, R.string.limits_total_mib, "policy-total", enabled = editable,
        keyboard = KeyboardOptions(keyboardType = KeyboardType.Decimal), update = editTotal)
    Text(stringResource(R.string.limits_used, PolicyUnits.usageMib(details.usage.attachmentBytes, locale), PolicyUnits.usageMib(details.usage.attachmentLimit, locale)),
        Modifier.testTag("policy-used"))
    Text(stringResource(R.string.limits_retention_note), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    if (details.usage.overLimit) Text(stringResource(R.string.limits_exceeded), Modifier.testTag("policy-over-limit"), color = MaterialTheme.colorScheme.error)
    if (details.usage.hasMissingContents || details.usage.unknownReferences) Text(stringResource(R.string.limits_incomplete),
        Modifier.testTag("policy-usage-incomplete"), color = MaterialTheme.colorScheme.error)
    Text(stringResource(R.string.limits_other_usage, PolicyUnits.usageMib(details.usage.retainedBytes, locale),
        PolicyUnits.usageMib(details.usage.draftBytes, locale), PolicyUnits.usageMib(details.usage.fileBytes, locale)),
        Modifier.testTag("policy-other-usage"), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}
