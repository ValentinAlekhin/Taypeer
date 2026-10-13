package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.LocalForm

/** Displays native form identities without loading their entered values. */
@Composable
internal fun LocalDraftsScreen(
    forms: List<LocalForm>,
    writable: Boolean,
    busy: Boolean = false,
    resume: (LocalForm) -> Unit,
    discard: (LocalForm) -> Unit,
    close: () -> Unit,
    error: Boolean = false,
) {
    var selected by remember { mutableStateOf<LocalForm?>(null) }
    val reviewed = selected?.let { chosen ->
        forms.firstOrNull { it.draft == chosen.draft && it.revision == chosen.revision }
    }
    val enabled = writable && !busy
    BackHandler {
        if (!busy) {
            if (reviewed != null) selected = null else close()
        }
    }
    LaunchedEffect(forms, selected) {
        if (selected != null && reviewed == null) selected = null
    }

    Scaffold(
        modifier = Modifier.testTag("drafts-screen"),
        topBar = { PhoneBar(stringResource(R.string.local_forms), back = { if (!busy) close() }) },
    ) { padding ->
        LazyColumn(
            Modifier.fillMaxSize().padding(padding).testTag("local-drafts-list"),
            contentPadding = PaddingValues(bottom = 16.dp),
        ) {
            item {
                Text(
                    stringResource(R.string.local_drafts_description),
                    Modifier.padding(16.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (!writable) item {
                Text(
                    stringResource(R.string.read_only), Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (forms.isEmpty()) item {
                Text(stringResource(R.string.no_local_drafts), Modifier.padding(16.dp).testTag("drafts-empty"))
            }
            if (error) item {
                Text(stringResource(R.string.operation_failed), Modifier.padding(16.dp).testTag("drafts-error"), color = MaterialTheme.colorScheme.error)
            }
            items(forms, key = { it.draft }) { form ->
                LocalDraftRow(form, enabled, resume = { resume(form) }, discard = { selected = form.copy() })
            }
            if (busy) item { LinearProgressIndicator(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) }
        }
    }

    reviewed?.let { form ->
        AlertDialog(
            onDismissRequest = { if (!busy) selected = null },
            modifier = Modifier.testTag("discard-draft-confirmation"),
            properties = DialogProperties(dismissOnBackPress = !busy, dismissOnClickOutside = !busy),
            title = { Text(stringResource(R.string.discard)) },
            text = { Text(stringResource(R.string.discard_local_draft_message, stringResource(draftLabel(form.kind)))) },
            confirmButton = {
                TextButton(
                    { selected = null; discard(form) },
                    enabled = enabled,
                    modifier = Modifier.testTag("confirm-discard-draft"),
                ) { Text(stringResource(R.string.discard), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(
                    { selected = null }, enabled = !busy, modifier = Modifier.testTag("cancel-discard-draft"),
                ) { Text(stringResource(R.string.cancel)) }
            },
        )
    }
}

@Composable
private fun LocalDraftRow(form: LocalForm, enabled: Boolean, resume: () -> Unit, discard: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Row(
                Modifier.weight(1f).heightIn(min = 64.dp).testTag("local-draft-${form.draft}")
                    .clickable(enabled = enabled, role = Role.Button, onClick = resume)
                    .padding(start = 16.dp, top = 12.dp, bottom = 12.dp),
                verticalAlignment = Alignment.Top,
            ) {
                Glyph(draftIcon(form.kind), Modifier.padding(top = 8.dp, end = 16.dp))
                Column(Modifier.weight(1f)) {
                    Text(stringResource(draftLabel(form.kind)), style = MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.Medium))
                    Text(
                        stringResource(if (form.dirty) R.string.unsaved_changes else R.string.save_clean),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Glyph(R.drawable.ic_chevron_right, Modifier.padding(start = 8.dp, top = 10.dp))
            }
            IconAction(
                R.drawable.ic_trash_2, R.string.discard, Modifier.testTag("discard-local-draft-${form.draft}"),
                enabled = enabled, action = discard,
            )
        }
        HorizontalDivider(Modifier.padding(start = 52.dp, end = 8.dp))
    }
}

private fun draftLabel(kind: FormKind): Int = when (kind) {
    FormKind.ENTRY -> R.string.entry
    FormKind.NEW_ENTRY -> R.string.new_entry
    FormKind.GROUP -> R.string.group
    FormKind.NEW_GROUP -> R.string.new_group
    FormKind.DATABASE -> R.string.database_info
}

private fun draftIcon(kind: FormKind): Int = when (kind) {
    FormKind.ENTRY, FormKind.NEW_ENTRY -> R.drawable.ic_key_round
    FormKind.GROUP, FormKind.NEW_GROUP -> R.drawable.ic_folder
    FormKind.DATABASE -> R.drawable.ic_database
}
