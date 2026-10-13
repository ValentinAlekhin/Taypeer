package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

@Composable
internal fun documentIconActions(state: DocumentState, file: () -> Unit): IconPickerActions {
    val target = state.iconTarget()
    return IconPickerActions(state.storedFavicon, state::loadStoredFavicon, state::selectStoredFavicon,
        { state.downloadIcon(it) }, file, state.icons.attempt(target), state.icons.failure(target),
        state.icons.confirmation(target), state::retryIcon)
}

/** A picker result is accepted explicitly after the exact database and draft are resumed. */
@Composable
@OptIn(ExperimentalLayoutApi::class)
internal fun PendingIcon(state: DocumentState) {
    val file = state.icons.file
    val target = state.iconTarget()
    val attempt = state.icons.attempt(target)
    if (file?.uri != null) Column(Modifier.padding(16.dp).testTag("pending-icon-file")) {
        Text(stringResource(if (state.overview?.database != file.target.database) R.string.unlock_selected_database
            else if (target != file.target) R.string.resume_selected_draft else R.string.selected_icon_ready))
        FlowRow {
            TextButton(state::acceptIconFile, enabled = target == file.target && attempt == null && !state.editingBlocked && state.overview?.writable == true,
                modifier = Modifier.testTag("accept-selected-icon")) { Text(stringResource(R.string.use_selected_icon)) }
            TextButton(state::discardIconFile, enabled = state.icons.attempt(file.target) == null) { Text(stringResource(R.string.cancel)) }
        }
    }
    if (attempt?.busy == true) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("icon-stage-busy"))
    if (attempt?.uncertain == true) Column(Modifier.padding(16.dp)) {
        Text(stringResource(R.string.icon_outcome_unknown), Modifier.testTag("pending-icon-unknown"), color = MaterialTheme.colorScheme.error)
        FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-icon"), enabled = state.overview?.writable == true, action = state::retryIcon)
    }
    if (state.icons.rejected(target)) Text(stringResource(R.string.invalid_form), Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error)
    state.icons.failure(target)?.let { failure ->
        Text(stringResource(iconFailureString(failure)), Modifier.padding(16.dp).testTag("icon-stage-error"), color = MaterialTheme.colorScheme.error)
    }
}
