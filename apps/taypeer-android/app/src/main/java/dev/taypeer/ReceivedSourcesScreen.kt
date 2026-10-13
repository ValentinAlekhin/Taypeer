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
import dev.taypeer.bridge.EntryRow
import dev.taypeer.bridge.GroupRow
import dev.taypeer.bridge.ChangedObject

/** Viewing a causal source never grants it network admission or applies it to the current document. */
@Composable
internal fun ReceivedSourcesScreen(state: ReceivedSourcesState, groups: List<GroupRow>, writable: Boolean,
    copy: (String) -> Unit, openExtracted: (ChangedObject) -> Unit) {
    var destination by remember(state.selected, state.entry?.id) { mutableStateOf<String?>(null) }
    var extracting by remember(state.selected, state.entry?.id) { mutableStateOf(false) }
    var confirmingDiscard by remember(state.selected) { mutableStateOf(false) }
    val canEdit = writable && !state.busy
    val back = { if (!state.busy) { if (extracting) extracting = false else state.back() } }
    BackHandler(onBack = back)
    Scaffold(topBar = {
        PhoneBar(stringResource(if (state.selected == null) R.string.received_sources else if (extracting && state.group != null) R.string.extract_group_source else if (extracting) R.string.extract_source else R.string.received_source), back = back) {
            if (state.selected == null) IconAction(R.drawable.ic_refresh_cw, R.string.refresh, enabled = !state.busy, action = state::refresh)
        }
    }, bottomBar = {
        if (extracting) FormActions({ if (!state.busy) extracting = false }, {
            state.extract(destination)
        }, enabled = canEdit && (destination != null || state.group != null || state.attempt != null) && state.extracted == null,
            label = if (state.attempt != null) R.string.retry_source else R.string.extract_action,
            tag = "extract-received-source", cancelEnabled = !state.busy)
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp)
            .testTag("received-sources-screen"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (state.selected == null) {
                Text(stringResource(R.string.received_sources_note), color = MaterialTheme.colorScheme.onSurfaceVariant)
                state.unresolvedRequests.forEach { request ->
                    FlatAction(R.string.retry_source, Modifier.fillMaxWidth().testTag("retry-source-${request.source.id}"), enabled = !state.busy) { state.retryExtraction(request) }
                }
                state.extracted?.let { changed ->
                    Text(stringResource(R.string.received_extracted), Modifier.testTag("received-extracted"))
                    FlatAction(if (changed.kind == dev.taypeer.bridge.FormKind.GROUP) R.string.open_extracted_group else R.string.open_extracted,
                        Modifier.fillMaxWidth().testTag("open-extracted-entry")) { state.close(); openExtracted(changed) }
                }
                if (state.rows.isEmpty() && !state.busy) Text(stringResource(R.string.received_sources_empty), Modifier.padding(vertical = 32.dp))
                state.rows.forEach { row ->
                    NavigationRow(stringResource(R.string.received_source_id, row.id.take(12)), R.drawable.ic_history,
                        Modifier.testTag("received-source-${row.id}"), stringResource(R.string.received_author, row.author.take(12))) {
                        if (!state.busy) state.inspect(row)
                    }
                }
            } else if (extracting) {
                Text(state.entry?.title ?: state.group?.name.orEmpty())
                Text(stringResource(if (state.group == null) R.string.received_extract_note else R.string.received_extract_group_note), color = MaterialTheme.colorScheme.onSurfaceVariant)
                GroupDestinationPicker(groups, if (state.attempt != null) state.attempt!!.group else destination, allowRoot = state.group != null,
                    enabled = canEdit && state.attempt == null && state.extracted == null) { destination = it }
                state.extracted?.let { id ->
                    Text(stringResource(R.string.received_extracted), Modifier.testTag("received-extracted"))
                    FlatAction(if (state.group == null) R.string.open_extracted else R.string.open_extracted_group, Modifier.fillMaxWidth().testTag("open-extracted-entry")) {
                        state.close(); openExtracted(id)
                    }
                }
            } else {
                Text(stringResource(R.string.received_author, state.selected!!.author.take(12)), color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(stringResource(R.string.received_causal_note), color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (state.entries.isEmpty() && state.group == null) Text(stringResource(R.string.received_no_entries))
                if (state.entries.size > 1) state.entries.forEach { row ->
                    NavigationRow(row.title, R.drawable.ic_key_round, Modifier.testTag("source-entry-${row.id}")) { state.selectEntry(row) }
                }
                state.entry?.let { entry ->
                    if (state.passwordVariants.size > 1) {
                        Text(stringResource(R.string.received_password_choice))
                        state.passwordVariants.forEachIndexed { index, variant ->
                            Row {
                                RadioButton(state.passwordVariant == variant, { state.selectPassword(variant) }, enabled = !state.busy)
                                TextButton({ state.selectPassword(variant) }, enabled = !state.busy) {
                                    Text(stringResource(R.string.received_password_variant, index + 1))
                                }
                            }
                        }
                    }
                    ReceivedEntryContent(entry, state.revealed, state.busy,
                        entry.hasPassword == true || state.passwordVariants.isNotEmpty(),
                        state.selected is SourceRow.Received || state.passwordVariant != null, state::reveal, copy) { state.copyPassword(copy) }
                    FlatAction(R.string.extract_source, Modifier.fillMaxWidth().testTag("choose-source-destination"), enabled = canEdit) { extracting = true }
                }
                state.group?.let { group ->
                    ValueRow(R.string.name, group.name)
                    group.description?.let { ValueRow(R.string.description, it) }
                    FlatAction(R.string.extract_group_source, Modifier.fillMaxWidth().testTag("choose-source-destination"), enabled = canEdit) { extracting = true }
                }
                if (state.selected is SourceRow.Received) TextButton({ confirmingDiscard = true }, Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("discard-received-source"), enabled = canEdit && state.attempt == null) {
                    Text(stringResource(R.string.discard_source), Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.error)
                }
            }
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (state.error) Text(stringResource(R.string.operation_failed), Modifier.testTag("received-source-error"), color = MaterialTheme.colorScheme.error)
        }
    }
    if (confirmingDiscard) AlertDialog(onDismissRequest = { if (!state.busy) confirmingDiscard = false },
        modifier = Modifier.testTag("discard-source-confirmation"), title = { Text(stringResource(R.string.discard_source)) },
        text = { Text(stringResource(R.string.received_discard_warning)) },
        confirmButton = { TextButton({ confirmingDiscard = false; state.discard() }, enabled = canEdit,
            modifier = Modifier.testTag("confirm-discard-source")) { Text(stringResource(R.string.discard_action)) } },
        dismissButton = { TextButton({ confirmingDiscard = false }, enabled = !state.busy) { Text(stringResource(R.string.cancel)) } })
}

@Composable
private fun ReceivedEntryContent(entry: EntryRow, revealed: String?, busy: Boolean, hasPassword: Boolean, passwordReady: Boolean, reveal: () -> Unit,
    copy: (String) -> Unit, copyPassword: () -> Unit) {
    ValueRow(R.string.title, entry.title)
    ValueRow(R.string.username, entry.username ?: "—") {
        entry.username?.let { value -> IconAction(R.drawable.ic_copy, R.string.copy) { copy(value) } }
    }
    ValueRow(R.string.password, if (hasPassword) revealed ?: "••••••••••••" else "—", secret = true) {
        if (hasPassword) {
            IconAction(if (revealed == null) R.drawable.ic_eye else R.drawable.ic_eye_off,
                if (revealed == null) R.string.reveal else R.string.hide,
                Modifier.testTag("reveal-source-password"), enabled = !busy && passwordReady, action = reveal)
            IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-source-password"), enabled = !busy && passwordReady, action = copyPassword)
        }
    }
    ValueRow(R.string.url, entry.url ?: "—") { entry.url?.let { value -> IconAction(R.drawable.ic_copy, R.string.copy) { copy(value) } } }
    ValueRow(R.string.notes, entry.notes ?: "—")
}
