package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

/** Transport lifetime and durable application are separate native observations. */
@Composable
internal fun ExchangeStatus(exchange: ExchangeView, application: ApplicationState?) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(if (exchange.running) R.string.exchange_running else R.string.exchange_stopped), Modifier.testTag("exchange-running"))
        Text(stringResource(R.string.encrypted_exchange_note), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        application?.let { observed ->
            val message = when (observed.status) {
                ApplicationPhase.FAILED -> stringResource(R.string.application_failed)
                ApplicationPhase.PENDING -> stringResource(R.string.application_pending, observed.pending.toString())
                ApplicationPhase.APPLIED -> if (observed.applied > 0uL) stringResource(R.string.application_applied, observed.applied.toString()) else stringResource(R.string.application_no_pending)
                ApplicationPhase.IDLE -> stringResource(R.string.application_waiting)
            }
            Text(message, Modifier.testTag("application-status"), style = MaterialTheme.typography.bodySmall,
                color = if (observed.status == ApplicationPhase.FAILED) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
internal fun DeviceExchangeStatus(device: DeviceRow) {
    Text(stringResource(when (device.lastExchangeSucceeded) {
        true -> R.string.device_exchange_succeeded
        false -> R.string.device_exchange_failed
        null -> R.string.device_exchange_unknown
    }), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}
