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
import java.util.Locale

private fun sortLabel(value: EntrySort): Int = when (value) {
    EntrySort.TITLE_ASC -> R.string.sort_title_ascending
    EntrySort.TITLE_DESC -> R.string.sort_title_descending
    EntrySort.USERNAME_ASC -> R.string.sort_username_ascending
    EntrySort.MODIFIED_NEWEST -> R.string.sort_modified_newest
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun EntrySortSheet(selected: EntrySort, select: (EntrySort) -> Unit, close: () -> Unit) {
    ModalBottomSheet(close, Modifier.testTag("entry-sort-sheet")) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(stringResource(R.string.sort_entries), Modifier.padding(start = 8.dp, bottom = 12.dp),
                style = MaterialTheme.typography.titleMedium)
            EntrySort.entries.forEach { value ->
                FlatAction(sortLabel(value), Modifier.fillMaxWidth().testTag("sort-${value.name.lowercase(Locale.ROOT)}")
                    .semantics { role = Role.RadioButton; this.selected = value == selected }, ghost = value != selected) { select(value) }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}
