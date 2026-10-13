package dev.taypeer

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.font.FontWeight
import dev.taypeer.bridge.*

@Composable
internal fun WelcomeContent(catalog: List<WorkingCopyView>, create: () -> Unit, open: () -> Unit, receive: () -> Unit, choose: (WorkingCopyView) -> Unit) {
    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Spacer(Modifier.height(12.dp))
        Text(stringResource(R.string.welcome_heading), style = MaterialTheme.typography.titleMedium)
        Text(stringResource(R.string.welcome_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(4.dp))
        FlatAction(R.string.create_database, Modifier.fillMaxWidth().testTag("create-database"), action = create)
        FlatAction(R.string.open_database, Modifier.fillMaxWidth().testTag("import-database"), action = open)
        FlatAction(R.string.receive_database, Modifier.fillMaxWidth().testTag("receive-database"), action = receive)
        if (catalog.isNotEmpty()) {
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            Text(stringResource(R.string.recent_databases))
            catalog.forEach { item ->
                NavigationRow(item.database.take(12), R.drawable.ic_database, Modifier.testTag("catalog-${item.database}"), stringResource(R.string.working_copy_locked)) { choose(item) }
            }
        }
    }
}

@Composable
internal fun EntriesContent(overview: DocumentOverview, selectedGroup: String?, query: String, search: (String) -> Unit, addGroup: () -> Unit, addEntry: () -> Unit, select: (String) -> Unit, sort: EntrySort = EntrySort.TITLE_ASC, iconPreview: (@Composable (EntryRow) -> Unit)? = null, contextActions: ((String) -> Unit)? = null, searchScope: (@Composable () -> Unit)? = null) {
    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SearchInput(query, search)
        searchScope?.invoke()
        val rows = overview.entries.filter { query.isNotEmpty() || selectedGroup == null || it.group == selectedGroup }
            .sortedWith(sort.comparator({ overview.database }, { it }))
        if (!overview.writable) Text(stringResource(R.string.read_only), style = MaterialTheme.typography.bodySmall)
        if (overview.groups.isEmpty()) {
            Spacer(Modifier.height(48.dp))
            Text(stringResource(R.string.no_groups))
            Text(stringResource(R.string.create_group_first), Modifier.testTag("create-group-first"), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            FlatAction(R.string.add_group, Modifier.fillMaxWidth().testTag("add-group"), enabled = overview.writable, action = addGroup)
        } else if (rows.isEmpty()) {
            Spacer(Modifier.height(48.dp))
            Text(stringResource(if (query.isEmpty()) R.string.no_entries else R.string.no_search_results))
            if (query.isNotEmpty()) Text(stringResource(R.string.search_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Column {
            rows.forEach { row -> androidx.compose.runtime.key(overview.database, row.id) {
                NavigationRow(row.title, savedIconDrawable(row.icon, R.drawable.ic_key_round), Modifier.testTag("entry-${row.id}"), row.username,
                    leading = iconPreview?.let { render -> { render(row) } }, onLongClick = contextActions?.let { open -> { open(row.id) } }) { select(row.id) }
            } }
        }
        Spacer(Modifier.height(72.dp))
    }
}

@Composable
@OptIn(ExperimentalFoundationApi::class)
internal fun NavigationRow(title: String, icon: Int, modifier: Modifier = Modifier, subtitle: String? = null, chevron: Boolean = true, titleMaxLines: Int = 2, subtitleMaxLines: Int = 2, leading: (@Composable () -> Unit)? = null, onLongClick: (() -> Unit)? = null, action: (() -> Unit)?) {
    val interaction = when {
        action == null -> Modifier
        onLongClick != null -> Modifier.combinedClickable(onClick = action, onLongClickLabel = stringResource(R.string.actions), onLongClick = onLongClick)
        else -> Modifier.clickable(onClick = action)
    }
    Box(modifier.fillMaxWidth().then(interaction)) {
        Row(Modifier.fillMaxWidth().heightIn(min = if (subtitle != null) 69.dp else 64.dp).padding(start = 8.dp, end = 4.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.Top) {
            Box(Modifier.padding(top = 8.dp)) { if (leading != null) leading() else Glyph(icon) }
            Column(Modifier.weight(1f).padding(start = 16.dp, end = 12.dp)) {
                Text(title, style = MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.Medium), maxLines = titleMaxLines, overflow = TextOverflow.Ellipsis)
                subtitle?.let { Text(it, style = MaterialTheme.typography.bodySmall.copy(lineHeight = 21.sp), color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = subtitleMaxLines, overflow = TextOverflow.Ellipsis) }
            }
            if (action != null && chevron) Glyph(R.drawable.ic_chevron_right, Modifier.padding(top = 10.dp))
        }
        HorizontalDivider(Modifier.align(Alignment.BottomStart).padding(start = 44.dp, end = 8.dp))
    }
}

@Composable
@OptIn(ExperimentalFoundationApi::class)
internal fun DatabaseDrawer(
    overview: DocumentOverview, catalog: List<WorkingCopyView>, selectedGroup: String?, forms: List<LocalForm>,
    selectGroup: (String?) -> Unit, editGroup: (String) -> Unit, addGroup: () -> Unit,
    chooseDatabase: (WorkingCopyView) -> Unit, create: () -> Unit, open: () -> Unit, receive: () -> Unit,
    exchange: () -> Unit, drafts: () -> Unit, settings: () -> Unit, lock: () -> Unit, trash: (() -> Unit)? = null, received: (() -> Unit)? = null, groupIcon: (@Composable (GroupRow, Int) -> Unit)? = null, unlockedCopies: List<WorkingCopyView> = emptyList(),
) {
    var databases by remember { mutableStateOf(false) }
    var collapsed by remember { mutableStateOf(setOf<String>()) }
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 8.dp, vertical = 8.dp)) {
        Row(Modifier.fillMaxWidth().clickable { databases = !databases }.heightIn(min = 64.dp).padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Glyph(R.drawable.ic_database)
            Column(Modifier.weight(1f).padding(start = 16.dp)) {
                Text(overview.name, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(stringResource(R.string.working_copy), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Glyph(if (databases) R.drawable.ic_chevron_down else R.drawable.ic_chevron_right)
        }
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        if (databases) {
            catalog.filter { it.database != overview.database }.forEach { item ->
                val open = unlockedCopies.any { it.database == item.database && it.path == item.path }
                NavigationRow(item.database.take(12), R.drawable.ic_database, Modifier.testTag("switch-${item.database}"),
                    stringResource(if (open) R.string.working_copy_open else R.string.working_copy_locked)) { chooseDatabase(item) }
            }
            FlatAction(R.string.create_database, Modifier.fillMaxWidth(), action = create)
            FlatAction(R.string.open_database, Modifier.fillMaxWidth(), action = open)
            FlatAction(R.string.receive_database, Modifier.fillMaxWidth(), action = receive)
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
        }
        fun visible(group: GroupRow): Boolean {
            var parent = group.parent
            val visited = mutableSetOf<String>()
            while (parent != null && visited.add(parent)) {
                if (parent in collapsed) return false
                parent = overview.groups.firstOrNull { it.id == parent }?.parent
            }
            return true
        }
        fun depth(group: GroupRow): Int {
            var parent = group.parent; var level = 0
            val visited = mutableSetOf<String>()
            while (parent != null && visited.add(parent)) { level++; parent = overview.groups.firstOrNull { it.id == parent }?.parent }
            return level.coerceAtMost(6)
        }
        // Stable preorder keeps children adjacent even when the native projection is flat.
        val ordered = mutableListOf<GroupRow>()
        fun append(parent: String?) {
            overview.groups.filter { it.parent == parent && it !in ordered }.forEach { group -> ordered.add(group); append(group.id) }
        }
        append(null)
        ordered.addAll(overview.groups.filter { it !in ordered })
        ordered.filter(::visible).forEach { group ->
            Row(Modifier.fillMaxWidth().background(if (group.id == selectedGroup) LocalSelectionColor.current else MaterialTheme.colorScheme.surface).padding(start = (depth(group) * 20).dp), verticalAlignment = Alignment.CenterVertically) {
                if (overview.groups.any { it.parent == group.id }) IconAction(if (group.id in collapsed) R.drawable.ic_chevron_right else R.drawable.ic_chevron_down, R.string.expand_group, Modifier.testTag("expand-${group.id}")) {
                    collapsed = if (group.id in collapsed) collapsed - group.id else collapsed + group.id
                } else Spacer(Modifier.width(48.dp))
                Row(Modifier.weight(1f).heightIn(min = 48.dp).testTag("select-group-${group.id}").combinedClickable(
                    onClick = { selectGroup(group.id) }, onLongClickLabel = stringResource(R.string.actions),
                    onLongClick = { editGroup(group.id) }), verticalAlignment = Alignment.CenterVertically) {
                    val fallback = savedIconDrawable(group.icon, if (group.id in collapsed) R.drawable.ic_folder else R.drawable.ic_folder_open)
                    if (groupIcon != null) groupIcon(group, fallback) else Glyph(fallback)
                    Text(group.name, Modifier.padding(start = 12.dp), maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
                IconAction(R.drawable.ic_ellipsis, R.string.edit_group, Modifier.testTag("edit-group-${group.id}"), enabled = overview.writable) { editGroup(group.id) }
            }
        }
        FlatAction(R.string.add_group, Modifier.fillMaxWidth().testTag("drawer-add-group"), enabled = overview.writable, action = addGroup)
        HorizontalDivider(Modifier.padding(vertical = 12.dp))
        NavigationRow(stringResource(R.string.devices_exchange), R.drawable.ic_monitor, Modifier.testTag("drawer-exchange"), chevron = false, action = exchange)
        trash?.let { NavigationRow(stringResource(R.string.trash_title), R.drawable.ic_trash_2, Modifier.testTag("drawer-trash"), chevron = false, action = it) }
        received?.let { NavigationRow(stringResource(R.string.received_sources), R.drawable.ic_history, Modifier.testTag("drawer-received-sources"), chevron = false, action = it) }
        NavigationRow(stringResource(R.string.local_forms) + if (forms.isEmpty()) "" else " · ${forms.size}", R.drawable.ic_history, Modifier.testTag("drawer-drafts"), chevron = false, action = drafts)
        NavigationRow(stringResource(R.string.settings), R.drawable.ic_sliders_horizontal, Modifier.testTag("drawer-settings"), chevron = false, action = settings)
        NavigationRow(stringResource(R.string.lock), R.drawable.ic_lock, Modifier.testTag("drawer-lock-database"), chevron = false, action = lock)
    }
}
