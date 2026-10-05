package dev.taypeer

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import dev.taypeer.bridge.AttachmentRow

class AttachmentActions(val select: (String?) -> Unit, val export: (String, String?, String, String) -> Unit, val importDatabase: () -> Unit)

/** Keep registrations outside the secret subtree: an OS picker always backgrounds this Activity. */
@Composable
fun rememberAttachmentActions(state: DocumentState): AttachmentActions {
    var importing by remember { mutableStateOf((state.attachmentSelection as? AttachmentSelection.Import)?.takeIf { it.uri == null }?.operation) }
    var exporting by remember { mutableStateOf((state.attachmentSelection as? AttachmentSelection.Export)?.takeIf { it.uri == null }?.operation) }
    val input = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> state.selectedAttachment(importing, uri); importing = null }
    val output = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri -> state.selectedAttachment(exporting, uri); exporting = null }
    val database = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> state.selectDatabaseImport(uri) }
    return AttachmentActions({ replacement ->
        if (importing != null || exporting != null) state.reportFailure()
        else state.prepareAttachment(replacement) { operation -> importing = operation; input.launch(arrayOf("*/*")) }
    }, { entry, revision, blob, name ->
        if (importing != null || exporting != null) state.reportFailure()
        else state.prepareExport(entry, revision, blob) { operation -> exporting = operation; output.launch(name) }
    }, { database.launch(arrayOf("application/octet-stream", "application/x-taypeer", "*/*")) })
}

/** UI selects a provider; bounded reads, validation, quotas and encryption stay in Rust. */
@Composable
@OptIn(ExperimentalLayoutApi::class)
fun Attachments(state: DocumentState, actions: AttachmentActions, rows: List<AttachmentRow>, entry: String?, revision: String? = null, editing: Boolean = false) {
    var rename by remember { mutableStateOf<AttachmentRow?>(null) }
    Text(stringResource(R.string.attachments), style = MaterialTheme.typography.titleMedium)
    if (rows.isEmpty()) Text(stringResource(R.string.no_attachments))
    rows.forEach { row ->
        Column(Modifier.fillMaxWidth().testTag("attachment-${row.attachment}")) {
            Text(row.name)
            row.contents.forEach { content ->
                Text(content.bytes?.let { "$it ${stringResource(R.string.bytes)}" } ?: stringResource(R.string.waiting_attachment))
                if (entry != null && content.bytes != null) TextButton({
                    actions.export(entry, revision, content.blob, row.name)
                }) { Text(stringResource(R.string.download_attachment)) }
            }
            if (editing) FlowRow {
                TextButton({ rename = row }, enabled = !state.navigating) { Text(stringResource(R.string.rename)) }
                TextButton({ actions.select(row.attachment) }, enabled = !state.navigating) { Text(stringResource(R.string.replace_file)) }
                TextButton({ state.removeAttachment(row.attachment) }, enabled = !state.navigating) { Text(stringResource(R.string.remove)) }
            }
        }
    }
    if (editing) TextButton({ actions.select(null) }, enabled = !state.navigating, modifier = Modifier.testTag("add-attachment")) {
        Text(stringResource(R.string.add_attachment))
    }
    rename?.let { row ->
        var name by remember(row.attachment) { mutableStateOf(row.name) }
        AlertDialog(onDismissRequest = { rename = null }, title = { Text(stringResource(R.string.rename)) },
            text = { TextField(name, { name = it }, label = { Text(stringResource(R.string.name)) }, modifier = Modifier.testTag("attachment-name")) },
            confirmButton = { TextButton({ state.renameAttachment(row.attachment, name); rename = null }, enabled = name.isNotEmpty()) { Text(stringResource(R.string.apply)) } },
            dismissButton = { TextButton({ rename = null }) { Text(stringResource(R.string.cancel)) } })
    }
}

/** A returned picker URI never applies itself to the session that replaced its old generation. */
@Composable
@OptIn(ExperimentalLayoutApi::class)
fun PendingAttachment(state: DocumentState) {
    val pending = state.attachmentSelection ?: return
    if (pending.uri == null) return
    val sameDatabase = state.overview?.database == pending.database
    val expectedDraft = when (pending) { is AttachmentSelection.Import -> pending.draft; is AttachmentSelection.Export -> pending.draft }
    val exactForm = expectedDraft == null || state.activeForm?.draft == expectedDraft
    Text(stringResource(if (!sameDatabase) R.string.unlock_selected_database else if (!exactForm) R.string.resume_selected_draft else R.string.selected_file_ready))
    FlowRow {
        TextButton(state::acceptAttachmentSelection, enabled = sameDatabase && exactForm && !state.navigating && (pending !is AttachmentSelection.Import || state.overview?.writable == true), modifier = Modifier.testTag("accept-selected-file")) {
            Text(stringResource(if (pending is AttachmentSelection.Import) R.string.add_selected_file else R.string.export_selected_attachment))
        }
        TextButton(state::discardAttachmentSelection) { Text(stringResource(R.string.cancel)) }
    }
}
