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

/** One immutable request is retained until the supervised Rust session confirms publication. */
@Composable
internal fun PasswordRotationScreen(state: DocumentState, revoke: String?, close: () -> Unit) {
    var attempt by remember { mutableStateOf<Pair<String, String>?>(null) }
    var busy by remember { mutableStateOf(false) }
    PasswordChangeScreen(revoke != null, busy, state.error, onSubmit = { password ->
        if (!busy) {
            val request = attempt ?: (password to newOperation()).also { attempt = it }
            busy = true
            state.rotate(request.first, revoke, request.second) { outcome ->
                busy = false
                when (outcome) {
                    RotationOutcome.CONFIRMED -> close()
                    RotationOutcome.REJECTED -> attempt = null
                    RotationOutcome.UNCERTAIN -> Unit
                }
            }
        }
    }, onClose = { if (!busy && attempt == null) { state.clearError(); close() } }, retained = attempt != null)
}

/** Secret input stays in this composition; the caller owns the native request and its outcome. */
@Composable
internal fun PasswordChangeScreen(
    revoking: Boolean,
    busy: Boolean,
    error: Boolean,
    onSubmit: (String) -> Unit,
    onClose: () -> Unit,
    retained: Boolean = false,
) {
    var password by remember(revoking) { mutableStateOf("") }
    var repeated by remember(revoking) { mutableStateOf("") }
    var revealed by remember(revoking, LocalSecretMaskEpoch.current) { mutableStateOf(false) }
    var repeatRevealed by remember(revoking, LocalSecretMaskEpoch.current) { mutableStateOf(false) }
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val editable = !busy && !retained
    val matching = password == repeated
    fun close() {
        if (!busy && !retained) {
            focus.clearFocus(force = true)
            keyboard?.hide()
            onClose()
        }
    }
    BackHandler { close() }
    LaunchedEffect(busy, retained) {
        if (busy || retained) { revealed = false; repeatRevealed = false }
    }

    Scaffold(
        modifier = Modifier.testTag("password-change-screen"),
        topBar = { PhoneBar(stringResource(if (revoking) R.string.revoke_device_title else R.string.password_change_title), back = ::close) },
        bottomBar = {
            FormActions(
                cancel = ::close,
                save = {
                    focus.clearFocus(force = true)
                    keyboard?.hide()
                    onSubmit(password)
                },
                enabled = !busy && password.isNotEmpty() && matching,
                tag = "apply-password-change",
                label = if (revoking) R.string.revoke_device_action else R.string.password_change_action,
                cancelEnabled = !busy && !retained,
                primary = true,
            )
        },
    ) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            PlainField(
                password, R.string.new_master_password, "rotation-new-password", enabled = editable,
                transformation = if (revealed && editable) VisualTransformation.None else PasswordVisualTransformation(),
                keyboard = KeyboardOptions(keyboardType = KeyboardType.Password),
                actions = {
                    IconAction(
                        if (revealed) R.drawable.ic_eye_off else R.drawable.ic_eye,
                        if (revealed) R.string.hide else R.string.reveal,
                        Modifier.testTag("reveal-rotation-password"), enabled = editable,
                    ) { revealed = !revealed }
                },
                update = { password = it },
            )
            PlainField(
                repeated, R.string.repeat_master_password, "rotation-repeat-password", enabled = editable,
                transformation = if (repeatRevealed && editable) VisualTransformation.None else PasswordVisualTransformation(),
                keyboard = KeyboardOptions(keyboardType = KeyboardType.Password),
                actions = {
                    IconAction(
                        if (repeatRevealed) R.drawable.ic_eye_off else R.drawable.ic_eye,
                        if (repeatRevealed) R.string.hide else R.string.reveal,
                        Modifier.testTag("reveal-rotation-repeat"), enabled = editable,
                    ) { repeatRevealed = !repeatRevealed }
                },
                update = { repeated = it },
            )
            if (repeated.isNotEmpty() && !matching) Text(
                stringResource(R.string.rotation_password_mismatch),
                Modifier.testTag("rotation-password-mismatch"), color = MaterialTheme.colorScheme.error,
            )
            Text(
                stringResource(if (revoking) R.string.revoke_device_warning else R.string.rotation_warning),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("password-change-busy"))
            if (error) Text(
                stringResource(if (retained) R.string.rotation_retry_retained else R.string.operation_failed),
                Modifier.testTag("password-change-error"), color = MaterialTheme.colorScheme.error,
            )
        }
    }
}
