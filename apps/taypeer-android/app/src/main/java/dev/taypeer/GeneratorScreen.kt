package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp

@Composable
fun GeneratorScreen(state: GeneratorState, copy: (String) -> Unit) {
    Column(Modifier.safeDrawingPadding().imePadding().padding(24.dp)
        .verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineLarge)
        Text(stringResource(R.string.runtime_stage))
        HorizontalDivider()
        Text(stringResource(R.string.generator), style = MaterialTheme.typography.titleLarge)
        Toggle(R.string.passphrase, state.passphrase) { state.passphrase = it }
        if (state.passphrase) {
            OutlinedTextField(state.words, { state.words = it }, label = { Text(stringResource(R.string.words)) },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), singleLine = true)
            OutlinedTextField(state.separator, { state.separator = it }, label = { Text(stringResource(R.string.separator)) })
        } else {
            OutlinedTextField(state.length, { state.length = it }, modifier = Modifier.testTag("length"), label = { Text(stringResource(R.string.length)) },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), singleLine = true)
            Toggle(R.string.uppercase, state.uppercase) { state.uppercase = it }
            Toggle(R.string.lowercase, state.lowercase) { state.lowercase = it }
            Toggle(R.string.digits, state.digits) { state.digits = it }
            Toggle(R.string.punctuation, state.punctuation) { state.punctuation = it }
            Toggle(R.string.exclude_similar, state.excludeSimilar) { state.excludeSimilar = it }
            OutlinedTextField(state.exclusions, { state.exclusions = it }, label = { Text(stringResource(R.string.exclusions)) })
        }
        Button(enabled = !state.running, onClick = state::generate) { Text(stringResource(R.string.generate)) }
        if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
        if (state.generated.isNotEmpty()) {
            OutlinedTextField(state.generated, {}, readOnly = true,
                visualTransformation = if (state.revealed) VisualTransformation.None else PasswordVisualTransformation(),
                modifier = Modifier.fillMaxWidth().testTag("generated-password"))
            TextButton(onClick = { state.revealed = !state.revealed }) { Text(stringResource(if (state.revealed) R.string.hide else R.string.reveal)) }
            Button(onClick = { copy(state.generated) }) { Text(stringResource(R.string.copy)) }
        }
    }
}
@Composable
private fun Toggle(label: Int, checked: Boolean, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(stringResource(label), modifier = Modifier.weight(1f))
        Switch(checked, update)
    }
}
