package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

@Composable
internal fun SearchScopeControl(scope: SearchScope, currentName: String?, choose: () -> Unit) {
    Button(choose, Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("search-scope"),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.surfaceVariant,
            contentColor = MaterialTheme.colorScheme.onSurface), contentPadding = PaddingValues(12.dp)) {
        Text(when {
            scope == SearchScope.ALL_UNLOCKED -> stringResource(R.string.search_all_unlocked)
            currentName != null -> stringResource(R.string.search_current_named, currentName)
            else -> stringResource(R.string.search_current)
        }, Modifier.weight(1f))
        Glyph(R.drawable.ic_chevron_down)
    }
}

/** The same real builder is used by the product and FIG surface fixtures. */
@Composable
internal fun SearchContent(query: String, scope: SearchScope, currentName: String?, rows: List<SearchResultRow>,
    busy: Boolean, failed: Boolean, update: (String) -> Unit, chooseScope: () -> Unit,
    retry: () -> Unit, select: (SearchResultRow) -> Unit, preview: (@Composable (SearchResultRow) -> Unit)? = null) {
    Column(Modifier.fillMaxWidth().padding(16.dp).testTag("search-results"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SearchInput(query, update)
        SearchScopeControl(scope, currentName, chooseScope)
        if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("search-loading"))
        if (failed) {
            Text(stringResource(R.string.search_incomplete), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("search-error"))
            FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-search"), enabled = !busy, action = retry)
        }
        if (!busy && rows.isEmpty() && !failed) {
            Spacer(Modifier.height(36.dp))
            Text(stringResource(R.string.no_search_results))
            Text(stringResource(R.string.search_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Column {
            rows.forEach { row -> androidx.compose.runtime.key(row.sourceKey, row.entry.id) {
                val source = stringResource(R.string.search_result_source, row.databaseName, row.groupName)
                val subtitle = row.entry.username?.takeIf(String::isNotEmpty)?.let { "$it\n$source" } ?: source
                NavigationRow(row.entry.title, savedIconDrawable(row.entry.icon, R.drawable.ic_key_round),
                    Modifier.testTag("search-entry-${row.copy.database}-${row.entry.id}"), subtitle,
                    subtitleMaxLines = Int.MAX_VALUE, leading = preview?.let { render -> { render(row) } }) { select(row) }
            } }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SearchScopeSheet(scope: SearchScope, select: (SearchScope) -> Unit, close: () -> Unit) {
    ModalBottomSheet(close, Modifier.testTag("search-scope-sheet")) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(stringResource(R.string.search_scope), Modifier.padding(start = 8.dp, bottom = 12.dp), style = MaterialTheme.typography.titleMedium)
            listOf(SearchScope.CURRENT to R.string.search_current, SearchScope.ALL_UNLOCKED to R.string.search_all_unlocked).forEach { (value, label) ->
                FlatAction(label, Modifier.fillMaxWidth().testTag(if (value == SearchScope.CURRENT) "search-current" else "search-all-unlocked")
                    .semantics { role = Role.RadioButton; selected = value == scope }, ghost = value != scope) { select(value) }
            }
            Text(stringResource(R.string.search_scope_help), Modifier.padding(8.dp), style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(24.dp))
        }
    }
}
