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
internal fun BiometrySettings(state: BiometryUiState, enable: () -> Unit, disable: () -> Unit, retry: () -> Unit) {
    val status = state.status
    Column(Modifier.fillMaxWidth().testTag("biometric-settings"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.biometric_setting), style = MaterialTheme.typography.titleSmall)
        Text(stringResource(when (status?.state) {
            BiometryState.ENABLED -> R.string.biometric_enabled
            BiometryState.NEEDS_MASTER -> R.string.biometric_needs_master
            BiometryState.PENDING -> R.string.biometric_pending
            BiometryState.DISABLED -> R.string.biometric_disabled
            null -> R.string.biometric_checking
        }), Modifier.testTag("biometric-status"), style = MaterialTheme.typography.bodyMedium)
        if (state.failed) Text(stringResource(R.string.biometric_failed), Modifier.testTag("biometric-error"), color = MaterialTheme.colorScheme.error)
        if (status?.availability != null && status.availability != BiometricAvailability.AVAILABLE)
            Text(stringResource(R.string.biometric_unavailable))
        if (state.retryOperation != null && status?.state != BiometryState.PENDING)
            FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("biometric-retry"), enabled = !state.busy, action = retry)
        when (status?.state) {
            BiometryState.ENABLED, BiometryState.NEEDS_MASTER -> FlatAction(R.string.biometric_disable,
                Modifier.fillMaxWidth().testTag("biometric-disable"), enabled = !state.busy, action = disable)
            BiometryState.PENDING -> FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("biometric-retry"), enabled = !state.busy, action = retry)
            BiometryState.DISABLED -> FlatAction(R.string.biometric_enable, Modifier.fillMaxWidth().testTag("biometric-enable"),
                enabled = !state.busy && state.retryOperation == null && status.availability == BiometricAvailability.AVAILABLE, action = enable)
            null -> Unit
        }
        if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("biometric-busy"))
    }
}
