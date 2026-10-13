package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

enum class ObjectAction { CLONE, MOVE }
data class ObjectActionSelection(val kind: FormKind, val target: String, val action: ObjectAction, val name: String, val destination: String?)
data class TrashReviewUi(val row: TrashRow, val review: LifecycleReview, val restoring: Boolean, val operation: String)

@Composable
internal fun GroupDestinationPicker(groups: List<GroupRow>, selected: String?, allowRoot: Boolean, enabled: Boolean, select: (String?) -> Unit) {
    Column {
        if (allowRoot) Row {
            RadioButton(selected == null, { select(null) }, enabled = enabled)
            NavigationRow(stringResource(R.string.top_level_groups), R.drawable.ic_database, Modifier.weight(1f).testTag("destination-root")) { if (enabled) select(null) }
        }
        groups.forEach { group ->
            Row {
                RadioButton(selected == group.id, { if (enabled) select(group.id) }, enabled = enabled)
                NavigationRow(group.name, R.drawable.ic_folder, Modifier.weight(1f).testTag("destination-${group.id}"), groupPath(group, groups)) { if (enabled) select(group.id) }
            }
        }
    }
}

@Composable
internal fun ObjectActionScreen(selection: ObjectActionSelection, groups: List<GroupRow>, busy: Boolean, error: Boolean, confirm: (String?, String, String) -> Unit, close: () -> Unit,
    request: ObjectActionRequest? = null, writable: Boolean = true) {
    var name by remember(selection) { mutableStateOf(selection.name) }
    var destination by remember(selection) { mutableStateOf(selection.destination) }
    val displayedName = request?.name ?: name
    val displayedDestination = if (request != null) request.destination else destination
    val operation = remember(selection, name, destination, request?.operation) { newOperation() }
    val cloning = selection.action == ObjectAction.CLONE
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val title = when {
        cloning && selection.kind == FormKind.ENTRY -> R.string.clone_entry
        cloning -> R.string.clone_group
        selection.kind == FormKind.ENTRY -> R.string.move_entry
        else -> R.string.move_group
    }
    BackHandler { if (!busy) close() }
    Scaffold(topBar = { PhoneBar(stringResource(title), back = { if (!busy) close() }) }, bottomBar = {
        FormActions({ if (!busy) close() }, { focus.clearFocus(force = true); keyboard?.hide(); confirm(displayedDestination, displayedName, request?.operation ?: operation) }, enabled = writable && !busy && (selection.kind != FormKind.ENTRY || displayedDestination != null) && (!cloning || displayedName.isNotBlank()), label = if (cloning) R.string.clone_action else R.string.move_action, tag = "confirm-object-action", cancelEnabled = !busy)
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp).testTag("object-action-screen"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (cloning) PlainField(displayedName, R.string.name, "clone-name", enabled = writable && !busy && request == null, update = { name = it })
            else Text(selection.name)
            Text(stringResource(R.string.destination_group), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            GroupDestinationPicker(groups, displayedDestination, selection.kind == FormKind.GROUP, writable && !busy && request == null) { destination = it }
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("object-action-error"))
            if (request != null) Text(stringResource(R.string.unconfirmed_operation), style = MaterialTheme.typography.bodySmall)
        }
    }
}

/** Replays only immutable requests already submitted to the shared document. */
@Composable
internal fun UnconfirmedOperations(state: DocumentState) {
    val lifecycle = state.lifecycle
    val restorations = state.restorations
    if (lifecycle.unresolvedActions.isEmpty() && restorations.unresolvedRequests.isEmpty()) return
    Column(Modifier.fillMaxWidth().padding(16.dp).testTag("unconfirmed-operations"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.unconfirmed_operation), style = MaterialTheme.typography.bodySmall)
        lifecycle.unresolvedActions.forEach { request ->
            val label = when {
                request.action == ObjectAction.CLONE && request.kind == FormKind.ENTRY -> R.string.clone_entry
                request.action == ObjectAction.CLONE -> R.string.clone_group
                request.kind == FormKind.ENTRY -> R.string.move_entry
                else -> R.string.move_group
            }
            NavigationRow(request.name, if (request.kind == FormKind.GROUP) R.drawable.ic_folder else R.drawable.ic_key_round,
                Modifier.testTag("retry-object-action-${request.operation}"), stringResource(label)) {
                if (!lifecycle.busy && !state.navigating && state.overview?.writable == true) lifecycle.retry(request)
            }
        }
        restorations.unresolvedRequests.forEach { request ->
            FlatAction(R.string.retry_restore_version, Modifier.fillMaxWidth().testTag("retry-restoration-${request.operation}"),
                enabled = !restorations.busy && !state.navigating && state.overview?.writable == true) { restorations.retry(request) }
        }
        if (lifecycle.busy || restorations.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if (lifecycle.error || restorations.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TrashScreen(rows: List<TrashRow>, groups: List<GroupRow>, review: TrashReviewUi?, busy: Boolean, error: Boolean,
    restore: (TrashRow, String?) -> Unit, purge: (TrashRow) -> Unit, confirmReview: () -> Unit, cancelReview: () -> Unit, refresh: () -> Unit, close: () -> Unit, writable: Boolean = true) {
    var selected by remember { mutableStateOf<TrashRow?>(null) }
    var destinationRow by remember { mutableStateOf<TrashRow?>(null) }
    var destination by remember { mutableStateOf<String?>(null) }
    BackHandler {
        if (!busy) when {
            review != null -> cancelReview()
            destinationRow != null -> destinationRow = null
            else -> close()
        }
    }
    LaunchedEffect(rows) { selected = selected?.takeIf { chosen -> rows.any { it.target == chosen.target && it.generation == chosen.generation } } }
    if (destinationRow != null) {
        val row = destinationRow!!
        Scaffold(topBar = { PhoneBar(stringResource(R.string.restore_from_trash), back = { if (!busy) destinationRow = null }) }, bottomBar = {
            FormActions({ if (!busy) destinationRow = null }, { restore(row, destination); destinationRow = null }, enabled = writable && !busy && (row.kind == FormKind.GROUP || destination != null), label = R.string.restore_action, tag = "prepare-trash-restore", cancelEnabled = !busy)
        }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Text(row.name)
                Text(stringResource(R.string.destination_group), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                GroupDestinationPicker(groups, destination, row.kind == FormKind.GROUP, writable && !busy) { destination = it }
            }
        }
    } else Scaffold(topBar = { PhoneBar(stringResource(R.string.trash_title), back = { if (!busy) close() }) { IconAction(R.drawable.ic_refresh_cw, R.string.refresh, enabled = !busy, action = refresh) } }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp).testTag("trash-screen")) {
            if (rows.isEmpty()) Text(stringResource(R.string.trash_empty), Modifier.padding(vertical = 48.dp))
            rows.forEach { row -> NavigationRow(row.name, if (row.kind == FormKind.GROUP) R.drawable.ic_folder else R.drawable.ic_key_round,
                Modifier.testTag("trash-${row.target}").background(if (row.target == selected?.target) LocalSelectionColor.current else MaterialTheme.colorScheme.surface), stringResource(if (row.kind == FormKind.GROUP) R.string.group else R.string.entry)) { if (!busy) selected = row } }
            selected?.let { row ->
                Spacer(Modifier.height(24.dp))
                FlatAction(R.string.restore_from_trash, Modifier.fillMaxWidth().testTag("choose-trash-restore"), enabled = writable && !busy) { destination = null; destinationRow = row }
                TextButton({ purge(row) }, Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("prepare-trash-purge"), enabled = writable && !busy) {
                    Text(stringResource(R.string.delete_permanently), Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.error)
                }
            }
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(vertical = 16.dp).testTag("trash-error"))
        }
    }
    review?.let { reviewed -> AlertDialog(onDismissRequest = { if (!busy) cancelReview() }, modifier = Modifier.testTag("trash-review"),
        title = { Text(stringResource(if (reviewed.restoring) R.string.restore_from_trash else R.string.delete_permanently)) },
        text = { Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.trash_review_counts, reviewed.review.groups.toString(), reviewed.review.entries.toString()))
            if (!reviewed.restoring) Text(stringResource(R.string.trash_purge_warning))
            if (error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
        } },
        confirmButton = { TextButton(confirmReview, enabled = !busy, modifier = Modifier.testTag("confirm-trash-review")) { Text(stringResource(if (reviewed.restoring) R.string.restore_action else R.string.delete_permanently)) } },
        dismissButton = { TextButton(cancelReview, enabled = !busy, modifier = Modifier.testTag("cancel-trash-review")) { Text(stringResource(R.string.cancel)) } }) }
}

private fun groupPath(group: GroupRow, groups: List<GroupRow>): String? {
    val names = mutableListOf<String>()
    val visited = mutableSetOf(group.id)
    var parent = group.parent
    while (parent != null && visited.add(parent)) {
        val ancestor = groups.firstOrNull { it.id == parent } ?: break
        names.add(ancestor.name); parent = ancestor.parent
    }
    return names.takeIf { it.isNotEmpty() }?.asReversed()?.joinToString(" / ")
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun EntryActionsSheet(entry: EntryRow, writable: Boolean, close: () -> Unit, edit: () -> Unit,
    clone: () -> Unit, move: () -> Unit, history: () -> Unit, trash: () -> Unit) {
    ModalBottomSheet(onDismissRequest = close, modifier = Modifier.testTag("entry-context-menu")) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(entry.title, style = MaterialTheme.typography.titleMedium)
            FlatAction(R.string.edit, Modifier.fillMaxWidth().testTag("context-edit-entry"), enabled = writable, action = edit)
            FlatAction(R.string.clone_entry, Modifier.fillMaxWidth().testTag("context-clone-entry"), enabled = writable, action = clone)
            FlatAction(R.string.move_entry, Modifier.fillMaxWidth().testTag("context-move-entry"), enabled = writable, action = move)
            FlatAction(R.string.history, Modifier.fillMaxWidth().testTag("context-entry-history"), action = history)
            TextButton(trash, Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("context-trash-entry"), enabled = writable) {
                Text(stringResource(R.string.trash), Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.error)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun GroupActionsSheet(group: GroupRow, writable: Boolean, close: () -> Unit, create: () -> Unit, edit: () -> Unit,
    clone: () -> Unit, move: () -> Unit, history: () -> Unit, favicons: () -> Unit, trash: () -> Unit) {
    ModalBottomSheet(onDismissRequest = close, modifier = Modifier.testTag("group-context-menu")) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(group.name, style = MaterialTheme.typography.titleMedium)
            FlatAction(R.string.new_subgroup, Modifier.fillMaxWidth().testTag("new-subgroup"), enabled = writable, action = create)
            FlatAction(R.string.edit, Modifier.fillMaxWidth().testTag("edit-selected-group"), enabled = writable, action = edit)
            FlatAction(R.string.clone_group, Modifier.fillMaxWidth().testTag("clone-group"), enabled = writable, action = clone)
            FlatAction(R.string.move_group, Modifier.fillMaxWidth().testTag("move-group"), enabled = writable, action = move)
            FlatAction(R.string.history, Modifier.fillMaxWidth().testTag("group-history"), action = history)
            FlatAction(R.string.bulk_favicon_title, Modifier.fillMaxWidth().testTag("download-group-favicons"), enabled = writable, action = favicons)
            TextButton(trash, Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("trash-group"), enabled = writable) {
                Text(stringResource(R.string.trash), Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.error)
            }
        }
    }
}
