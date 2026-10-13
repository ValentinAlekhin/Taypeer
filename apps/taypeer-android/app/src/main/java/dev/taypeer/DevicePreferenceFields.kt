package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.DeviceSettingsView

@Composable
internal fun DevicePreferenceFields(state: DeviceSettingsState) {
    LaunchedEffect(state) { if (state.settings == null && !state.busy) state.load() }
    DevicePreferencesContent(state.settings, state.busy, state.failed, state::applyIdle, state::applyClipboard, state::load)
}

/** Only acknowledged native values fill closed fields; uncertain writes retain the typed dialog. */
@Composable
internal fun DevicePreferencesContent(settings: DeviceSettingsView?, busy: Boolean, failed: Boolean,
    applyIdle: (UInt) -> Unit, applyClipboard: (UInt?) -> Unit, reload: () -> Unit) {
    var custom by remember { mutableStateOf<DurationSetting?>(null) }
    var input by remember { mutableStateOf("") }
    var attempted by remember { mutableStateOf(false) }
    val parsed = input.toUIntOrNull()
    val confirmed = when (custom) { DurationSetting.IDLE -> settings?.idleSeconds; DurationSetting.CLIPBOARD -> settings?.clipboardSeconds; null -> null }
    LaunchedEffect(confirmed, busy, failed, attempted) {
        if (attempted && !busy && !failed && parsed != null && confirmed == parsed) { custom = null; attempted = false }
    }
    val enabled = settings != null && !busy
    SelectorField(R.string.auto_lock, settings?.idleSeconds?.let { durationLabel(it) } ?: stringResource(R.string.settings_loading), "idle-selector", enabled) { dismiss ->
        listOf(60u, 300u, 900u, 3600u).forEach { seconds ->
            DropdownMenuItem({ Text(durationLabel(seconds)) }, { dismiss(); applyIdle(seconds) }, Modifier.testTag("idle-$seconds"))
        }
        DropdownMenuItem({ Text(stringResource(R.string.custom_duration)) }, { dismiss(); custom = DurationSetting.IDLE; input = settings!!.idleSeconds.toString(); attempted = false }, Modifier.testTag("idle-custom"))
    }
    SelectorField(R.string.clipboard_clear, settings?.let { it.clipboardSeconds?.let { seconds -> durationLabel(seconds) } ?: stringResource(R.string.do_not_clear) }
        ?: stringResource(R.string.settings_loading), "clipboard-selector", enabled) { dismiss ->
        listOf(15u, 30u, 60u, 120u).forEach { seconds ->
            DropdownMenuItem({ Text(durationLabel(seconds)) }, { dismiss(); applyClipboard(seconds) }, Modifier.testTag("clipboard-$seconds"))
        }
        DropdownMenuItem({ Text(stringResource(R.string.do_not_clear)) }, { dismiss(); applyClipboard(null) }, Modifier.testTag("clipboard-never"))
        DropdownMenuItem({ Text(stringResource(R.string.custom_duration)) }, { dismiss(); custom = DurationSetting.CLIPBOARD; input = settings!!.clipboardSeconds?.toString().orEmpty(); attempted = false }, Modifier.testTag("clipboard-custom"))
    }
    if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("device-settings-saving"))
    if (failed) {
        Text(stringResource(R.string.device_settings_unconfirmed), Modifier.testTag("device-settings-error"), color = MaterialTheme.colorScheme.error)
        FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("reload-device-settings"), enabled = !busy, action = reload)
    }
    custom?.let { setting ->
        AlertDialog(onDismissRequest = { if (!busy) { custom = null; attempted = false } }, modifier = Modifier.testTag("duration-dialog"),
            title = { Text(stringResource(if (setting == DurationSetting.IDLE) R.string.auto_lock else R.string.clipboard_clear)) },
            text = { Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                PlainField(input, R.string.duration_seconds, "duration-seconds", enabled = !busy, keyboard = KeyboardOptions(keyboardType = KeyboardType.Number), update = { input = it; attempted = false })
                if (input.isNotEmpty() && parsed == null) Text(stringResource(R.string.duration_number_required), color = MaterialTheme.colorScheme.error)
                if (attempted && failed) Text(stringResource(R.string.device_settings_unconfirmed), color = MaterialTheme.colorScheme.error)
            } },
            confirmButton = { TextButton({ attempted = true; parsed?.let { if (setting == DurationSetting.IDLE) applyIdle(it) else applyClipboard(it) } }, enabled = !busy && parsed != null, modifier = Modifier.testTag("apply-duration")) { Text(stringResource(R.string.apply)) } },
            dismissButton = { TextButton({ custom = null; attempted = false }, enabled = !busy, modifier = Modifier.testTag("cancel-duration")) { Text(stringResource(R.string.cancel)) } })
    }
}

private enum class DurationSetting { IDLE, CLIPBOARD }

@Composable
private fun durationLabel(seconds: UInt): String {
    val minutes = seconds / 60u
    return if (seconds > 0u && seconds % 60u == 0u)
        pluralStringResource(R.plurals.after_minutes, minutes.coerceAtMost(Int.MAX_VALUE.toUInt()).toInt(), minutes.toString())
    else pluralStringResource(R.plurals.after_seconds, seconds.coerceAtMost(Int.MAX_VALUE.toUInt()).toInt(), seconds.toString())
}
