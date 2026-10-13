package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

@Composable
internal fun GroupFaviconsScreen(state: GroupFaviconsState) {
    GroupFaviconsForm(state.group?.name.orEmpty(), state.recursive, state.replace, state.editable,
        state.busy, state.started, state.retryable, state.rows, state.failure,
        state::updateRecursive, state::updateReplace, state::submit, state::close)
}

/** Actual saved FIG composition, also used by deterministic public visual fixtures. */
@Composable
internal fun GroupFaviconsForm(group: String, recursive: Boolean, replace: Boolean, editable: Boolean,
    busy: Boolean, started: Boolean, retryable: Boolean, rows: List<GroupFaviconRow>, failure: GroupFaviconUiFailure?,
    updateRecursive: (Boolean) -> Unit, updateReplace: (Boolean) -> Unit, submit: () -> Unit, close: () -> Unit) {
    BackHandler(onBack = close)
    Scaffold(Modifier.testTag("bulk-favicon-screen"),
        topBar = { PhoneBar(stringResource(R.string.bulk_favicon_title), back = close) },
        bottomBar = { FormActions(close, submit, enabled = !busy && (!started || retryable || failure != null),
            tag = "load-group-favicons", label = if (started) R.string.retry else R.string.download_icon_action,
            cancelEnabled = true, primary = true) }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(stringResource(R.string.bulk_favicon_group, group), style = MaterialTheme.typography.bodyLarge)
            FaviconOption(R.string.bulk_favicon_recursive, "bulk-favicon-recursive", recursive, editable, updateRecursive)
            FaviconOption(R.string.bulk_favicon_replace, "bulk-favicon-replace", replace, editable, updateReplace)
            Text(stringResource(R.string.bulk_favicon_help), color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("bulk-favicon-busy"))
            if (failure != null) Text(stringResource(if (failure == GroupFaviconUiFailure.REJECTED) R.string.invalid_form else R.string.bulk_favicon_unconfirmed), Modifier.testTag("bulk-favicon-error"), color = MaterialTheme.colorScheme.error)
            rows.forEach { row ->
                Column(Modifier.fillMaxWidth().testTag("bulk-favicon-row-${row.entry}"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(row.title, style = MaterialTheme.typography.bodyLarge)
                    Text(stringResource(faviconStateString(row.state)), Modifier.testTag("bulk-favicon-status-${row.entry}"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    row.failure?.let { reason -> Text(stringResource(when (reason) {
                        is GroupFaviconFailure.Image -> iconFailureString(reason.failure)
                        GroupFaviconFailure.Rejected -> R.string.invalid_form
                        GroupFaviconFailure.Storage -> R.string.operation_failed
                        GroupFaviconFailure.Unconfirmed -> R.string.bulk_favicon_unconfirmed
                    }), color = MaterialTheme.colorScheme.error) }
                }
            }
        }
    }
}

@Composable
private fun FaviconOption(label: Int, tag: String, selected: Boolean, enabled: Boolean, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(tag)
        .toggleable(selected, enabled = enabled, role = Role.Checkbox, onValueChange = update), verticalAlignment = Alignment.CenterVertically) {
        Checkbox(selected, onCheckedChange = null, enabled = enabled)
        Text(stringResource(label), Modifier.padding(start = 8.dp))
    }
}

private fun faviconStateString(state: GroupFaviconState): Int = when (state) {
    GroupFaviconState.SKIPPED -> R.string.bulk_favicon_skipped
    GroupFaviconState.MISSING_URL -> R.string.bulk_favicon_missing_url
    GroupFaviconState.READY -> R.string.bulk_favicon_ready
    GroupFaviconState.RUNNING -> R.string.bulk_favicon_running
    GroupFaviconState.APPLIED -> R.string.bulk_favicon_applied
    GroupFaviconState.FAILED -> R.string.bulk_favicon_failed
    GroupFaviconState.UNCERTAIN -> R.string.bulk_favicon_unconfirmed
    GroupFaviconState.CANCELLED -> R.string.bulk_favicon_cancelled
}
