package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

/** Groups and database metadata keep their own real fields; no artificial entry form is built. */
@Composable
internal fun MetadataHistoryScreen(state: MetadataHistoryState, writable: Boolean, copy: (String) -> Unit,
    blocked: Boolean = false) {
    val selected = state.selection ?: return
    val value = state.view
    val retained = state.request()
    var parent by remember(selected) { mutableStateOf<String?>(null) }
    var parentChosen by remember(selected) { mutableStateOf(false) }
    LaunchedEffect(value?.revision) {
        if (!parentChosen && retained == null && value != null) {
            parent = value.saved.parent?.takeIf { id -> value.parentCandidates.any { it.id == id } }
        }
    }
    // A read ACK can arrive before its enclosing navigation finishes the current projection.
    // Keep all commands unavailable until that same owner releases its command gate.
    val busy = state.busy || blocked
    fun close() { if (!busy) state.close() }
    BackHandler { close() }
    MetadataHistoryContent(selected.kind, state.mode, value, state.savedPreview, state.currentPreview,
        if (retained != null) retained.parent else parent, retained != null, busy, state.error, writable,
        close = ::close, confirm = {
            if (retained != null) state.retry(retained)
            else state.confirm(parent, newOperation())
        }, retryRead = state::retryRead, compare = state::compare, showSaved = state::showSaved,
        prepareRestore = state::prepareRestore, chooseParent = { parent = it; parentChosen = true }, copy = copy)
}

/** The retained owner supplies exact callbacks; public visual fixtures reuse this same composition. */
@Composable
internal fun MetadataHistoryContent(kind: FormKind, mode: MetadataHistoryMode, value: MetadataRevisionView?,
    savedPreview: RenderedIcon?, currentPreview: RenderedIcon?, parent: String?, retained: Boolean,
    busy: Boolean, error: Boolean, writable: Boolean, close: () -> Unit, confirm: () -> Unit,
    retryRead: () -> Unit, compare: () -> Unit, showSaved: () -> Unit, prepareRestore: () -> Unit,
    chooseParent: (String?) -> Unit, copy: (String) -> Unit) {
    val restoring = mode == MetadataHistoryMode.RESTORE
    Scaffold(modifier = Modifier.testTag("metadata-history-screen"), topBar = {
        PhoneBar(stringResource(when (mode) {
            MetadataHistoryMode.COMPARE -> R.string.compare_versions
            MetadataHistoryMode.RESTORE -> R.string.restore_version
            MetadataHistoryMode.SAVED -> R.string.metadata_saved_version
        }), back = close)
    }, bottomBar = {
        if (restoring && writable) FormActions(close, confirm,
            !busy && (value != null || retained), "confirm-metadata-restore", R.string.restore_action,
            cancelEnabled = !busy)
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (error) {
                Text(stringResource(if (!retained) R.string.operation_failed else R.string.metadata_retry_retained),
                    Modifier.testTag("metadata-history-error"), color = MaterialTheme.colorScheme.error)
                if (!restoring) FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-metadata-history"),
                    enabled = !busy, action = retryRead)
            }
            value?.let { projected ->
                Text(stringResource(R.string.selected_version_caption, displayTime(projected.savedAt)),
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(stringResource(if (kind == FormKind.GROUP) R.string.metadata_group_snapshot else R.string.metadata_database_snapshot),
                    style = MaterialTheme.typography.titleMedium)
                when (mode) {
                    MetadataHistoryMode.SAVED, MetadataHistoryMode.RESTORE -> {
                        MetadataSnapshotContent(projected.saved, kind, projected.parentCandidates, savedPreview, "saved", copy)
                    }
                    MetadataHistoryMode.COMPARE -> {
                        Text(stringResource(R.string.selected_value), style = MaterialTheme.typography.titleMedium)
                        MetadataSnapshotContent(projected.saved, kind, projected.parentCandidates, savedPreview, "saved", copy)
                        HorizontalDivider()
                        Text(stringResource(R.string.current_version_caption), style = MaterialTheme.typography.titleMedium)
                        MetadataSnapshotContent(projected.current, kind, projected.parentCandidates, currentPreview, "current", copy)
                    }
                }
                if (restoring) {
                    if (kind == FormKind.GROUP) {
                        GroupParentField(parent, projected.parentCandidates, !busy && !retained, chooseParent)
                    }
                    Text(stringResource(R.string.metadata_restore_warning), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    if (mode == MetadataHistoryMode.SAVED) FlatAction(R.string.compare_version,
                        Modifier.fillMaxWidth().testTag("compare-metadata-version"), enabled = !busy, action = compare)
                    else FlatAction(R.string.view_version, Modifier.fillMaxWidth().testTag("view-metadata-version"),
                        enabled = !busy, action = showSaved)
                    if (writable) FlatAction(R.string.restore_version, Modifier.fillMaxWidth().testTag("restore-metadata-version"),
                        enabled = !busy, action = prepareRestore)
                }
            }
            if (value == null && retained && restoring) {
                Text(stringResource(R.string.metadata_retry_retained), style = MaterialTheme.typography.bodySmall)
                if (kind == FormKind.GROUP) Text(if (parent == null) stringResource(R.string.group_parent_root)
                    else stringResource(R.string.metadata_frozen_destination))
            }
        }
    }
}

@Composable
private fun MetadataSnapshotContent(value: MetadataSnapshotView, kind: FormKind,
    candidates: List<GroupParentCandidate>, preview: RenderedIcon?, side: String, copy: (String) -> Unit) {
    Column(Modifier.fillMaxWidth().testTag("metadata-$side-snapshot"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ValueRow(R.string.name, value.name) { IconAction(R.drawable.ic_copy, R.string.copy) { copy(value.name) } }
        ValueRow(R.string.description, value.description ?: "—") {
            value.description?.let { description -> IconAction(R.drawable.ic_copy, R.string.copy) { copy(description) } }
        }
        if (kind == FormKind.GROUP) {
            ValueRow(R.string.group_parent, if (value.parent == null) stringResource(R.string.group_parent_root)
                else candidates.firstOrNull { it.id == value.parent }?.name ?: stringResource(R.string.group_parent_unavailable))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                IconPreviewContent(preview, savedIconDrawable(value.icon, R.drawable.ic_folder), Modifier.testTag("metadata-$side-icon"))
                Text(stringResource(R.string.icon), style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

/** Direct retry remains reachable when the selected immutable source no longer exists. */
@Composable
internal fun MetadataRestoreRetries(state: MetadataHistoryState, enabled: Boolean) {
    state.unresolvedRequests.forEach { request ->
        FlatAction(R.string.metadata_retry_restore, Modifier.fillMaxWidth().testTag("retry-metadata-restore-${request.operation}"),
            enabled = enabled && !state.busy) { state.retry(request) }
    }
}
