package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.EntryProperties
import dev.taypeer.bridge.HistoryRow

/** Read-only native projections, shared by inspector/editor tabs and visual fixtures. */
@Composable
internal fun EntryPropertiesContent(value: EntryProperties?, copy: (String) -> Unit) {
    if (value == null) { Text(stringResource(R.string.no_saved_properties)); return }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
    val created = displayTime(value.createdAt)
    val modified = displayTime(value.modifiedAt)
    ValueRow(R.string.created_at, created) {
        IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-property-created")) { copy(created) }
    }
    ValueRow(R.string.modified_at, modified) {
        IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-property-modified")) { copy(modified) }
    }
    ValueRow(R.string.accessed_at, value.accessedAt?.let(::displayTime) ?: "—") {
        value.accessedAt?.let { time -> IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-property-access")) { copy(displayTime(time)) } }
    }
    ValueRow(R.string.object_id, value.entry) {
        IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-property-id")) { copy(value.entry) }
    }
    ValueRow(R.string.expiry, value.expiresAt?.let { displayTime(it) } ?: stringResource(R.string.never_expires))
    ValueRow(R.string.tags, value.tags.joinToString(", ").ifEmpty { "—" })
    }
}

@Composable
internal fun EntryHistoryContent(rows: List<HistoryRow>, writable: Boolean,
    initialSelection: String? = null, action: (HistoryRow, HistoryAction) -> Unit) {
    var selected by remember(rows.map { it.id }, initialSelection) { mutableStateOf(initialSelection) }
    var menu by remember { mutableStateOf(false) }
    val chosen = rows.firstOrNull { it.id == selected }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(stringResource(R.string.original_versions), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
        Box {
            IconAction(R.drawable.ic_ellipsis, R.string.actions, Modifier.testTag("history-actions")) { menu = true }
            DropdownMenu(menu, { menu = false }) {
                DropdownMenuItem({ Text(stringResource(R.string.purge_selected)) }, {
                    menu = false; chosen?.let { action(it, HistoryAction.PURGE) }
                }, Modifier.testTag("history-clear-version"), enabled = writable && chosen != null)
            }
        }
    }
    if (rows.isEmpty()) Text(stringResource(R.string.no_saved_versions))
    rows.forEach { row ->
        NavigationRow(displayTime(row.savedAt), R.drawable.ic_history,
            Modifier.background(if (chosen?.id == row.id) MaterialTheme.colorScheme.surfaceVariant else androidx.compose.ui.graphics.Color.Transparent)
                .testTag("history-version-${row.id}").semantics { this.selected = chosen?.id == row.id },
            subtitle = row.name, titleMaxLines = Int.MAX_VALUE, action = { selected = row.id })
    }
    FlatAction(R.string.view_version, Modifier.fillMaxWidth().testTag("history-view-version"), enabled = chosen != null) { chosen?.let { action(it, HistoryAction.VIEW) } }
    FlatAction(R.string.compare_version, Modifier.fillMaxWidth().testTag("history-compare-version"), enabled = chosen != null) { chosen?.let { action(it, HistoryAction.COMPARE) } }
    FlatAction(R.string.restore_version, Modifier.fillMaxWidth().testTag("history-restore-version"), enabled = writable && chosen != null) { chosen?.let { action(it, HistoryAction.RESTORE) } }
}

internal enum class HistoryAction { VIEW, COMPARE, RESTORE, PURGE }

@Composable
@OptIn(ExperimentalLayoutApi::class)
internal fun EditableAttributeRow(row: dev.taypeer.bridge.AttributeRow, value: String, protected: Boolean, enabled: Boolean,
    update: (String) -> Unit, protect: (Boolean) -> Unit, rename: () -> Unit, reveal: () -> Unit, remove: () -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        Text(row.name)
        TextField(value, update, modifier = Modifier.fillMaxWidth().testTag("attribute-value-${row.id}"), enabled = enabled,
            label = { Text(stringResource(R.string.value)) },
            placeholder = { if (protected && row.value == null) Text(stringResource(R.string.protected_value)) },
            visualTransformation = if (protected) androidx.compose.ui.text.input.PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.protected_value), Modifier.weight(1f))
            Checkbox(protected, protect, enabled = enabled, modifier = Modifier.testTag("attribute-protected-${row.id}"))
        }
        FlowRow {
            TextButton(rename, enabled = enabled) { Text(stringResource(R.string.rename)) }
            if (protected && row.id != null) TextButton(reveal, Modifier.testTag("reveal-editor-attribute-${row.id}")) { Text(stringResource(R.string.reveal)) }
            TextButton(remove, enabled = enabled) { Text(stringResource(R.string.remove)) }
        }
    }
}

/** Both live and saved inspectors render the same native attribute projection. */
@Composable
internal fun EntryAttributesContent(attributes: List<dev.taypeer.bridge.AttributeRow>, revealedAttribute: String?, revealed: String,
    reveal: (String) -> Unit, copyAttribute: (String) -> Unit, copy: (String) -> Unit, tagPrefix: String = "attribute",
    display: (String) -> SecretDisplayTicket? = { null }) {
    attributes.forEach { attribute ->
        Column(Modifier.testTag("$tagPrefix-${attribute.id}")) {
            Text(attribute.name, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
                val id = attribute.id
                val shown = if (attribute.protected) {
                    if (id != null && revealedAttribute == id) revealed.ifEmpty { "••••••••••••" } else "••••••••••••"
                } else attribute.value.orEmpty()
                val ticket = if (attribute.protected && id != null && revealedAttribute == id && revealed.isNotEmpty()) display(id) else null
                Text(shown, Modifier.weight(1f).secretDisplay(ticket))
                if (attribute.protected && id != null) {
                    IconAction(R.drawable.ic_eye, R.string.reveal, Modifier.testTag("reveal-$tagPrefix-$id")) { reveal(id) }
                    IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-$tagPrefix-$id")) { copyAttribute(id) }
                } else if (!attribute.protected) attribute.value?.let { value -> IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-$tagPrefix-$id")) { copy(value) } }
            }
            HorizontalDivider()
        }
    }
}
