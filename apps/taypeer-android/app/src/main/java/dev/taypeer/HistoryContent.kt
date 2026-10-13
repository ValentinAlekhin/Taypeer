package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.EntryRow
import dev.taypeer.bridge.GroupRow
import dev.taypeer.bridge.HistoryRow
import dev.taypeer.bridge.EntryProperties
import dev.taypeer.bridge.AttributeRow

/** Saved rows are native projections; captions never invent an actor or a change reason. */
@Composable
internal fun HistoryListContent(rows: List<HistoryRow>, selected: Set<String>, writable: Boolean,
    inspectable: Boolean, select: (HistoryRow) -> Unit, check: (String, Boolean) -> Unit, purge: () -> Unit) {
    Text(stringResource(R.string.original_versions), style = MaterialTheme.typography.titleMedium)
    if (rows.isEmpty()) Text(stringResource(R.string.history_empty), color = MaterialTheme.colorScheme.onSurfaceVariant)
    Column {
        rows.forEach { row ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (writable) Checkbox(row.id in selected, { check(row.id, it) }, Modifier.testTag("check-history-${row.id}"))
                NavigationRow(displayTime(row.savedAt), R.drawable.ic_history,
                    Modifier.weight(1f).testTag("history-version-${row.id}"), row.name, titleMaxLines = Int.MAX_VALUE,
                    action = if (inspectable) ({ select(row) }) else null)
            }
        }
    }
    if (writable) FlatAction(R.string.purge_selected, Modifier.fillMaxWidth().testTag("purge-selected-history"), enabled = selected.isNotEmpty(), action = purge)
}

@Composable
internal fun SavedRevisionScreen(row: HistoryRow, entry: EntryRow?, password: String?, error: Boolean,
    writable: Boolean, togglePassword: () -> Unit, copy: (String) -> Unit, copyPassword: () -> Unit,
    compare: () -> Unit, restore: () -> Unit, close: () -> Unit, additional: @Composable (() -> Unit)? = null,
    properties: EntryProperties? = null, selectedTab: Int = 0, selectTab: (Int) -> Unit = {},
    revisions: List<HistoryRow> = emptyList(), revisionAction: (HistoryRow, HistoryAction) -> Unit = { _, _ -> }, iconPreview: (@Composable () -> Unit)? = null,
    display: SecretDisplayTicket? = null) {
    BackHandler(onBack = close)
    Scaffold(modifier = Modifier.testTag("saved-version-screen"), topBar = {
        PhoneBar(stringResource(R.string.saved_version_title, displayTime(row.savedAt)), back = close, titleMaxLines = Int.MAX_VALUE)
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            EntryTabs(selectedTab, selectTab)
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Text(stringResource(R.string.saved_version_caption, row.name), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
                if (entry == null && !error) LinearProgressIndicator(Modifier.fillMaxWidth())
                entry?.let {
                    Column(Modifier.fillMaxWidth().testTag("saved-version-tab-$selectedTab"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        when (selectedTab) {
                            0 -> EntrySummary(it, properties?.tags.orEmpty(), properties?.expiresAt, password,
                                togglePassword, copy, copyPassword, showProperties = properties != null, display = display)
                            1 -> additional?.invoke()
                            2 -> {
                                if (properties != null) AppearanceSummary(properties.appearance, it.title, iconPreview)
                                else if (!error) LinearProgressIndicator(Modifier.fillMaxWidth())
                            }
                            3 -> {
                                if (properties != null) EntryPropertiesContent(properties, copy)
                                else if (!error) LinearProgressIndicator(Modifier.fillMaxWidth())
                            }
                            4 -> EntryHistoryContent(revisions, writable, row.id, revisionAction)
                        }
                    }
                    if (selectedTab != 4) {
                        FlatAction(R.string.compare_version, Modifier.fillMaxWidth().testTag("compare-version"), action = compare)
                        if (writable) FlatAction(R.string.restore_version, Modifier.fillMaxWidth().testTag("restore-version"), action = restore)
                    }
                }
            }
        }
    }
}

/** The saved scope has only explicit reveal/copy actions; editing belongs to the current form. */
@Composable
internal fun SavedAttributesContent(attributes: List<AttributeRow>, revealedAttribute: String?, revealed: String,
    reveal: (String) -> Unit, copyAttribute: (String) -> Unit, copy: (String) -> Unit, display: (String) -> SecretDisplayTicket? = { null }) {
    EntryAttributesContent(attributes, revealedAttribute, revealed, reveal, copyAttribute, copy, "saved-attribute", display)
}

@Composable
internal fun HistoryComparisonScreen(saved: EntryRow?, current: EntryRow?, row: HistoryRow,
    savedPassword: String?, currentPassword: String?, busy: Boolean, error: Boolean, writable: Boolean,
    reveal: (ComparisonSide) -> Unit, copyPassword: (ComparisonSide) -> Unit,
    retry: () -> Unit, close: () -> Unit, restore: () -> Unit, savedDisplay: SecretDisplayTicket? = null,
    currentDisplay: SecretDisplayTicket? = null) {
    BackHandler(onBack = close)
    Scaffold(modifier = Modifier.testTag("comparison-screen"), topBar = {
        PhoneBar(stringResource(R.string.compare_versions), back = close)
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (error) {
                Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
                FlatAction(R.string.retry, Modifier.fillMaxWidth(), enabled = !busy, action = retry)
            }
            if (saved != null && current != null) {
                Text(stringResource(R.string.comparison_caption, current.title), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(stringResource(R.string.password), style = MaterialTheme.typography.titleMedium)
                Text(stringResource(R.string.selected_version_caption, displayTime(row.savedAt)), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                ComparisonPassword(R.string.saved_password, "saved", saved.hasPassword == true, savedPassword, busy,
                    { reveal(ComparisonSide.SAVED) }, { copyPassword(ComparisonSide.SAVED) }, savedDisplay)
                Text(stringResource(R.string.current_version_caption), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                ComparisonPassword(R.string.current_password, "current", current.hasPassword == true, currentPassword, busy,
                    { reveal(ComparisonSide.CURRENT) }, { copyPassword(ComparisonSide.CURRENT) }, currentDisplay)
                // Equality of masked passwords says nothing about their protected values.
                ComparisonField(R.string.title, saved.title, current.title)
                ComparisonField(R.string.username, saved.username, current.username)
                ComparisonField(R.string.url, saved.url, current.url)
                ComparisonField(R.string.notes, saved.notes, current.notes)
                if (writable) FlatAction(R.string.restore_version, Modifier.fillMaxWidth().testTag("restore-compared-version"), enabled = !busy, action = restore)
            }
        }
    }
}

@Composable
private fun ComparisonPassword(label: Int, side: String, exists: Boolean, password: String?, busy: Boolean,
    reveal: () -> Unit, copy: () -> Unit, display: SecretDisplayTicket?) {
    Column(Modifier.semantics(mergeDescendants = true) {}.testTag("comparison-$side-password")) {
        ValueRow(label, if (exists) password ?: "••••••••••••" else "—", secret = true, display = if (password != null) display else null) {
            if (exists) {
                IconAction(if (password == null) R.drawable.ic_eye else R.drawable.ic_eye_off,
                    if (password == null) R.string.reveal else R.string.hide,
                    Modifier.testTag("reveal-comparison-$side"), enabled = !busy, action = reveal)
                IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-comparison-$side"), enabled = !busy, action = copy)
            }
        }
    }
}

@Composable
private fun ComparisonField(label: Int, saved: String?, current: String?) {
    if (saved == current) return
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(label), style = MaterialTheme.typography.titleMedium)
        Text(stringResource(R.string.selected_value), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(saved ?: "—")
        Text(stringResource(R.string.current_value), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(current ?: "—")
    }
}

@Composable
internal fun RestoreVersionScreen(row: HistoryRow, groups: List<GroupRow>, busy: Boolean, error: Boolean,
    retainedDestination: String?, retained: Boolean, confirm: (String) -> Unit, close: () -> Unit, initialDestination: String? = null) {
    var destination by remember(row.id) { mutableStateOf<String?>(null) }
    var expanded by remember { mutableStateOf(false) }
    LaunchedEffect(row.id, initialDestination) {
        if (!retained && destination == null && groups.any { it.id == initialDestination }) destination = initialDestination
    }
    val chosen = if (retained) retainedDestination else destination
    fun cancel() { if (!busy) close() }
    BackHandler { cancel() }
    Scaffold(modifier = Modifier.testTag("restore-version-screen"), topBar = {
        PhoneBar(stringResource(R.string.restore_version), back = ::cancel)
    }, bottomBar = {
        FormActions(::cancel, { chosen?.let(confirm) }, !busy && chosen != null,
            "confirm-restore-version", R.string.restore_action, cancelEnabled = !busy)
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(stringResource(R.string.saved_version_title, displayTime(row.savedAt)), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(row.name, style = MaterialTheme.typography.titleMedium)
            Box {
                ActionField(groups.firstOrNull { it.id == chosen }?.name ?: stringResource(R.string.choose_group),
                    R.string.destination_group, "history-destination", enabled = !busy && !retained) { expanded = true }
                DropdownMenu(expanded, { expanded = false }) {
                    groups.forEach { group -> DropdownMenuItem({ Text(group.name) }, { destination = group.id; expanded = false }, Modifier.testTag("history-destination-${group.id}")) }
                }
            }
            Text(stringResource(R.string.restore_version_warning), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (error) Text(stringResource(if (retained) R.string.history_retry_retained else R.string.operation_failed), Modifier.testTag("history-restore-error"), color = MaterialTheme.colorScheme.error)
        }
    }
}
