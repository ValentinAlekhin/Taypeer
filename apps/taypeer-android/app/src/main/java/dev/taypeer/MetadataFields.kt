package dev.taypeer

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.FormKind
import dev.taypeer.bridge.GroupParentCandidate

/** Native supplies the exact form's allowed IDs; presentation never derives a subtree. */
@Composable
internal fun GroupParentField(parent: String?, candidates: List<GroupParentCandidate>, enabled: Boolean, select: (String?) -> Unit) {
    val root = stringResource(R.string.group_parent_root)
    val label = if (parent == null) root else candidates.firstOrNull { it.id == parent }?.name
        ?: stringResource(R.string.group_parent_unavailable)
    SelectorField(R.string.group_parent, label, "metadata-parent", enabled) { dismiss ->
        DropdownMenuItem(text = { Text(root) }, modifier = Modifier.testTag("metadata-parent-root"),
            onClick = { select(null); dismiss() })
        candidates.forEach { candidate ->
            key(candidate.id) {
                DropdownMenuItem(text = { Text(candidate.name) }, modifier = Modifier.testTag("metadata-parent-${candidate.id}"),
                    onClick = { select(candidate.id); dismiss() })
            }
        }
    }
}

/** Shared by the actual route and PUBLIC visual fixtures. Save remains the route's one command. */
@Composable
internal fun MetadataFields(input: MetadataInput, enabled: Boolean, name: (String) -> Unit, description: (String) -> Unit,
    parent: (String?) -> Unit, icon: @Composable ColumnScope.() -> Unit = {}) {
    val group = input.source.form.kind == FormKind.GROUP || input.source.form.kind == FormKind.NEW_GROUP
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        PlainField(input.name, R.string.name, "metadata-name", enabled = enabled, update = name)
        PlainField(input.description, R.string.description, "metadata-description", enabled = enabled,
            singleLine = false, minLines = if (group) 3 else 1, update = description)
        if (group) {
            key(input.source.form.draft) {
                GroupParentField(input.parent, input.source.parentCandidates, enabled, parent)
            }
            icon()
        }
    }
}
