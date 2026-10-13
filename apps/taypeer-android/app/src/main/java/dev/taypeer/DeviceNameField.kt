package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction

@Composable
internal fun DeviceNameField(state: DeviceSettingsState) {
    LaunchedEffect(state) { if (state.settings == null && !state.busy) state.load() }
    DeviceNameContent(state.nameDraft, state.nameFailure, state::editName, state::submitName, state::finishNameInput)
}

/** A local raw draft; committing and classifying its error belong to the state/native owner. */
@Composable
internal fun DeviceNameContent(value: String?, failure: DeviceNameFailure?, edit: (String) -> Unit,
    submit: () -> Unit, blur: () -> Unit) {
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    var focused by remember { mutableStateOf(false) }
    fun finish() { submit(); focus.clearFocus(force = true); keyboard?.hide() }
    PlainField(value.orEmpty(), R.string.device_name, "device-name",
        placeholder = if (value == null) stringResource(R.string.settings_loading) else null,
        keyboard = KeyboardOptions(imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { finish() }),
        focusChanged = { now ->
            val previous = focused
            focused = now
            if (previous && !now) blur()
        },
        inputModifier = Modifier.onPreviewKeyEvent { event ->
            if (event.key == Key.Enter || event.key == Key.NumPadEnter) {
                if (event.type == KeyEventType.KeyUp) finish()
                true
            } else false
        }, update = edit)
    failure?.let {
        val message = when (it) {
            DeviceNameFailure.REQUIRED -> R.string.device_name_required
            DeviceNameFailure.TOO_LONG -> R.string.device_name_too_long
            DeviceNameFailure.PROFILE_IO -> R.string.device_name_io
            DeviceNameFailure.PROFILE_INVALID -> R.string.device_name_profile_invalid
            DeviceNameFailure.PROFILE_BUSY -> R.string.device_name_profile_busy
            DeviceNameFailure.UNCERTAIN -> R.string.device_name_unconfirmed
            DeviceNameFailure.OTHER -> R.string.device_name_failed
        }
        Text(stringResource(message), Modifier.testTag("device-name-error"), color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodySmall)
        FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-device-name"), action = submit)
    }
}
