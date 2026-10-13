package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

/** Registered outside secrecyEpoch: the genuine picker must still lock the document. */
@Composable
internal fun rememberCiphertextCopyPicker(state: CiphertextCopyState): () -> Unit {
    var request by rememberSaveable { mutableStateOf(state.pendingPicker?.request) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        val expected = request; request = null
        state.selected(expected, uri)
    }
    return {
        state.preparePicker()?.let { selected ->
            request = selected.request
            try { picker.launch(selected.filename) }
            catch (_: Exception) { request = null; state.pickerFailed(selected.request) }
        }
    }
}

/** The FIG copy panel contains no document fields or editable internal path. */
@Composable
internal fun CiphertextCopyScreen(state: CiphertextCopyState, choose: () -> Unit) {
    BackHandler(onBack = state::cancel)
    Scaffold(modifier = Modifier.testTag("ciphertext-copy-screen"),
        topBar = { PhoneBar(stringResource(R.string.save_database_copy), back = state::cancel) },
        bottomBar = { Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
            TextButton(state::cancel, Modifier.testTag("cancel-database-copy")) {
                Text(stringResource(if (state.phase == CiphertextCopyPhase.SAVED) R.string.close else R.string.cancel))
            }
        } }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            ReadOnlyField(state.filename, R.string.copy_filename, "database-copy-filename")
            if (state.phase == CiphertextCopyPhase.CAPTURING || state.phase == CiphertextCopyPhase.WRITING)
                LinearProgressIndicator(Modifier.fillMaxWidth().testTag("database-copy-progress"))
            FlatAction(R.string.choose_copy_location, Modifier.fillMaxWidth().testTag("choose-copy-location"),
                enabled = state.canChoose, action = choose)
            Text(stringResource(R.string.database_copy_note))
            if (state.phase == CiphertextCopyPhase.SAVED)
                Text(stringResource(R.string.database_copy_saved), Modifier.testTag("database-copy-saved"))
            state.failure?.let { failure ->
                Text(stringResource(when (failure) {
                    CiphertextCopyFailure.CAPTURE -> R.string.database_copy_capture_failed
                    CiphertextCopyFailure.OUTPUT -> R.string.database_copy_output_failed
                    CiphertextCopyFailure.UNCONFIRMED -> R.string.database_copy_unconfirmed
                }), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("database-copy-error"))
                if (failure == CiphertextCopyFailure.CAPTURE)
                    FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-copy-capture"), action = state::retryCapture)
            }
        }
    }
}
