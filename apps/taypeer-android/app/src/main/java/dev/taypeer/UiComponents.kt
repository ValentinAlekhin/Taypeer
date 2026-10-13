package dev.taypeer

import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

@Composable
internal fun Glyph(@DrawableRes icon: Int, modifier: Modifier = Modifier, tint: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    Icon(painterResource(icon), contentDescription = null, modifier = modifier.size(20.dp), tint = tint)
}

@Composable
internal fun IconAction(@DrawableRes icon: Int, label: Int, modifier: Modifier = Modifier, enabled: Boolean = true, action: () -> Unit) {
    IconButton(action, modifier.size(48.dp), enabled = enabled) {
        Icon(painterResource(icon), stringResource(label), Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
internal fun PhoneBar(title: String, back: (() -> Unit)? = null, menu: (() -> Unit)? = null, titleMaxLines: Int = 2, actions: @Composable RowScope.() -> Unit = {}) {
    Column {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            if (menu != null) IconAction(R.drawable.ic_menu, R.string.groups, Modifier.testTag("group-menu"), action = menu)
            else if (back != null) IconAction(R.drawable.ic_arrow_left, R.string.back, Modifier.testTag("navigation-back"), action = back)
            else Spacer(Modifier.width(48.dp))
            Text(title, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.titleLarge, maxLines = titleMaxLines, overflow = TextOverflow.Ellipsis)
            actions()
        }
        HorizontalDivider()
    }
}

/** The same visible chrome is retained by the product and synthetic screenshot harness. */
@Composable
internal fun LibraryHeader(title: String, databaseOpen: Boolean, editor: Boolean, inspecting: Boolean, metadataEditing: Boolean, settings: Boolean,
    back: () -> Unit, menu: () -> Unit, edit: () -> Unit, editEnabled: Boolean, openActions: () -> Unit, menuContent: @Composable () -> Unit = {}) {
    PhoneBar(title, back = if (settings || editor) back else null, menu = if (databaseOpen && !editor && !settings) menu else null) {
        if (inspecting && !settings) IconAction(R.drawable.ic_pencil, R.string.edit, Modifier.testTag("edit-entry"), enabled = editEnabled, action = edit)
        if (!metadataEditing && !settings) Box {
            IconAction(if (databaseOpen) R.drawable.ic_ellipsis else R.drawable.ic_sliders_horizontal, R.string.actions, Modifier.testTag("screen-actions"), action = openActions)
            menuContent()
        }
    }
}

@Composable
internal fun AddEntryFab(enabled: Boolean, action: () -> Unit) {
    Surface(Modifier.padding(end = 8.dp).size(56.dp), color = MaterialTheme.colorScheme.surfaceVariant, shape = RoundedCornerShape(6.dp)) {
        IconAction(R.drawable.ic_plus, R.string.add_entry, Modifier.fillMaxSize().testTag("add-entry"), enabled = enabled, action = action)
    }
}

@Composable
internal fun FlatAction(label: Int, modifier: Modifier = Modifier, primary: Boolean = false, enabled: Boolean = true, ghost: Boolean = false, action: () -> Unit) {
    Button(
        action, modifier.heightIn(min = 48.dp), enabled = enabled, shape = RoundedCornerShape(6.dp),
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 12.dp),
        colors = ButtonDefaults.buttonColors(
            containerColor = if (ghost) Color.Transparent else if (primary) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            contentColor = if (primary) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
            disabledContainerColor = if (ghost) Color.Transparent else MaterialTheme.colorScheme.onSurface.copy(alpha = .12f),
        ),
    ) { Text(stringResource(label), Modifier.fillMaxWidth()) }
}

/** Shared geometry for an input and its independent, non-overlapping actions. */
@Composable
internal fun PlainField(
    value: String, label: Int, tag: String, enabled: Boolean = true, singleLine: Boolean = true, minLines: Int = 1, maxLines: Int = Int.MAX_VALUE,
    structured: Boolean = false, transformation: VisualTransformation = VisualTransformation.None,
    keyboard: KeyboardOptions = KeyboardOptions.Default, placeholder: String? = null,
    actions: @Composable RowScope.() -> Unit = {},
    keyboardActions: androidx.compose.foundation.text.KeyboardActions = androidx.compose.foundation.text.KeyboardActions.Default,
    focusChanged: (Boolean) -> Unit = {}, inputModifier: Modifier = Modifier, update: (String) -> Unit,
) {
    val requester = remember { FocusRequester() }
    var focused by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth()) {
        Text(stringResource(label), Modifier.clickable(enabled = enabled) { requester.requestFocus() },
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(4.dp))
        val shape = RoundedCornerShape(4.dp)
        Row(
            Modifier.fillMaxWidth().heightIn(min = 48.dp)
                .background(if (structured) Color.Transparent else MaterialTheme.colorScheme.surfaceVariant, shape)
                .then(if (focused && !structured) Modifier.border(2.dp, MaterialTheme.colorScheme.onSurface, shape) else Modifier),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            BasicTextField(
                value, update, Modifier.weight(1f).heightIn(min = 48.dp).testTag(tag).focusRequester(requester)
                    .onFocusChanged { focused = it.isFocused; focusChanged(it.isFocused) }.then(inputModifier)
                    .padding(horizontal = if (structured) 0.dp else 12.dp, vertical = 12.dp),
                enabled = enabled, singleLine = singleLine, minLines = minLines, maxLines = maxLines, keyboardOptions = keyboard,
                keyboardActions = keyboardActions, visualTransformation = transformation,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.onSurface),
                decorationBox = { inner -> Box { if (value.isEmpty() && placeholder != null) Text(placeholder, color = MaterialTheme.colorScheme.onSurfaceVariant); inner() } },
            )
            actions()
        }
        if (structured) { Spacer(Modifier.height(8.dp)); HorizontalDivider() }
    }
}

@Composable
internal fun SearchInput(value: String, update: (String) -> Unit, placeholder: Int = R.string.search_entries, tag: String = "entry-search") {
    var focused by remember { mutableStateOf(false) }
    val label = stringResource(R.string.search)
    val shape = RoundedCornerShape(4.dp)
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).background(MaterialTheme.colorScheme.surfaceVariant, shape)
        .then(if (focused) Modifier.border(2.dp, MaterialTheme.colorScheme.onSurface, shape) else Modifier), verticalAlignment = Alignment.CenterVertically) {
        Glyph(R.drawable.ic_search, Modifier.padding(horizontal = 14.dp))
        BasicTextField(value, update, Modifier.weight(1f).testTag(tag).semantics { contentDescription = label }
            .onFocusChanged { focused = it.isFocused }.padding(vertical = 12.dp),
            singleLine = true, textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
            cursorBrush = SolidColor(MaterialTheme.colorScheme.onSurface),
            decorationBox = { inner -> Box { if (value.isEmpty()) Text(stringResource(placeholder), color = MaterialTheme.colorScheme.onSurfaceVariant); inner() } })
        if (value.isNotEmpty()) IconAction(R.drawable.ic_x, R.string.clear_search, Modifier.testTag("clear-search")) { update("") }
        else Spacer(Modifier.width(12.dp))
    }
}

@Composable
internal fun ValueRow(label: Int, value: String, secret: Boolean = false, display: SecretDisplayTicket? = null, actions: @Composable RowScope.() -> Unit = {}) {
    Column(Modifier.fillMaxWidth()) {
        Text(stringResource(label), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(4.dp))
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(value, Modifier.weight(1f).padding(vertical = 12.dp).secretDisplay(display), style = MaterialTheme.typography.bodyLarge.copy(fontFamily = if (secret) FontFamily.Monospace else FontFamily.SansSerif))
            actions()
        }
        Spacer(Modifier.height(8.dp))
        HorizontalDivider()
    }
}

@Composable
internal fun ReadOnlyField(value: String, label: Int, tag: String, minHeight: androidx.compose.ui.unit.Dp = 48.dp) {
    Column(Modifier.fillMaxWidth()) {
        Text(stringResource(label), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(4.dp))
        Box(Modifier.fillMaxWidth().heightIn(min = minHeight).background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(4.dp))
            .testTag(tag).padding(12.dp), contentAlignment = Alignment.CenterStart) {
            Text(value, style = MaterialTheme.typography.bodyLarge)
        }
    }
}

@Composable
internal fun ActionField(value: String, label: Int?, tag: String, enabled: Boolean = true,
    leading: @Composable (() -> Unit)? = null, action: () -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        if (label != null) {
            Text(stringResource(label), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(4.dp))
        }
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(tag)
            .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(4.dp))
            .clickable(enabled, role = Role.Button, onClick = action).padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            leading?.invoke()
            Text(value, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
            Glyph(R.drawable.ic_chevron_down, Modifier.padding(start = 12.dp))
        }
    }
}

@Composable
internal fun FormActions(cancel: () -> Unit, save: () -> Unit, enabled: Boolean, tag: String = "save-form", label: Int = R.string.save, status: @Composable (() -> Unit)? = null, cancelLabel: Int = R.string.cancel, cancelEnabled: Boolean = enabled, primary: Boolean = false) {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer).imePadding()) {
        HorizontalDivider()
        status?.invoke()
        val stacked = LocalDensity.current.fontScale >= 1.8f
        if (stacked) Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            FlatAction(cancelLabel, Modifier.fillMaxWidth().testTag("cancel-form"), enabled = cancelEnabled, ghost = true, action = cancel)
            FlatAction(label, Modifier.fillMaxWidth().testTag(tag), primary = primary, enabled = enabled, action = save)
        } else Row(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FlatAction(cancelLabel, Modifier.weight(1f).testTag("cancel-form"), enabled = cancelEnabled, ghost = true, action = cancel)
            FlatAction(label, Modifier.weight(1f).testTag(tag), primary = primary, enabled = enabled, action = save)
        }
    }
}
