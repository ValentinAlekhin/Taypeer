package dev.taypeer

import android.net.Uri
import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.BackHandler
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import android.view.ViewTreeObserver
import android.content.Context
import android.content.ContextWrapper
import dev.taypeer.bridge.*
import dev.taypeer.platform.ExchangeScheduler
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

private sealed interface Authentication {
    data object Create : Authentication
    data class Open(val copy: WorkingCopyView) : Authentication
    data class Import(val uri: Uri) : Authentication
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LibraryScreen(state: DocumentState, attachments: AttachmentActions, copy: (String) -> Unit) {
    key(state.secrecyEpoch) { LibraryContent(state, attachments, copy) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LibraryContent(state: DocumentState, attachments: AttachmentActions, copy: (String) -> Unit) {
    var authentication by remember { mutableStateOf<Authentication?>(null) }
    var join by remember { mutableStateOf(false) }
    var forms by remember { mutableStateOf(false) }
    val drawer = rememberDrawerState(DrawerValue.Closed)
    val scope = rememberCoroutineScope()
    val groupsLabel = stringResource(R.string.groups)
    val context = androidx.compose.ui.platform.LocalContext.current
    val notificationPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { state.exchange() }
    fun exchange() {
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        else state.exchange()
    }
    LaunchedEffect(state.databaseImport) {
        state.databaseImport?.let { authentication = Authentication.Import(it) }
    }
    BackHandler(enabled = state.history == null && state.exchange == null && (state.entry != null || state.metadata != null || state.inspectedEntry != null)) {
        state.chooseGroup(state.selectedGroup, state.ungrouped)
    }
    ModalNavigationDrawer(drawerState = drawer, gesturesEnabled = state.entry == null && state.metadata == null && state.inspectedEntry == null, drawerContent = {
        ModalDrawerSheet(Modifier.width(288.dp).verticalScroll(rememberScrollState())) {
            Text(stringResource(R.string.groups), Modifier.padding(16.dp), style = MaterialTheme.typography.titleLarge)
            TextButton({ state.chooseGroup(null); scope.launch { drawer.close() } }) { Text(stringResource(R.string.all_entries)) }
            TextButton({ state.chooseGroup(null, true); scope.launch { drawer.close() } }) { Text(stringResource(R.string.ungrouped)) }
            state.overview?.groups?.forEach { group ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    TextButton({ state.chooseGroup(group.id); scope.launch { drawer.close() } }, Modifier.weight(1f)) { Text(group.name) }
                    TextButton({ state.beginMetadata(FormKind.GROUP, group.id); scope.launch { drawer.close() } }, enabled = state.overview?.writable == true) { Text(stringResource(R.string.edit)) }
                }
            }
            TextButton({ state.beginMetadata(FormKind.NEW_GROUP); scope.launch { drawer.close() } }, enabled = state.overview?.writable == true) { Text(stringResource(R.string.add_group)) }
            HorizontalDivider()
            TextButton({ forms = true; scope.launch { drawer.close() } }) { Text(stringResource(R.string.local_forms)) }
            TextButton({ state.beginMetadata(FormKind.DATABASE); scope.launch { drawer.close() } }, enabled = state.overview?.writable == true) { Text(stringResource(R.string.database_info)) }
        }
    }) {
        Scaffold(topBar = {
            TopAppBar(title = { Text(state.overview?.name ?: stringResource(R.string.app_name)) }, navigationIcon = {
                if (state.overview != null) TextButton({ scope.launch { drawer.open() } }, Modifier.testTag("group-menu").semantics { contentDescription = groupsLabel }) { Text("☰") }
            }, actions = {
                TextButton({ state.saveNow(); state.generator = true }) { Text(stringResource(R.string.generator_short)) }
                TextButton(::exchange) { Text(stringResource(R.string.sync)) }
                if (state.overview != null) TextButton(state::lock) { Text(stringResource(R.string.lock)) }
            })
        }, bottomBar = {
            if (state.activeForm != null) Row(Modifier.fillMaxWidth().imePadding().padding(horizontal = 16.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(saveText(state.saveLabel)), Modifier.weight(1f).testTag("save-status"))
                if (state.saveLabel == SaveLabel.FAILED) TextButton(state::saveNow) { Text(stringResource(R.string.retry)) }
            }
        }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding)) {
            if (state.entry != null || state.inspectedEntry != null) EntryTabs(state)
            Column(Modifier.weight(1f).padding(horizontal = 16.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("document-error"))
                PendingAttachment(state)
                if (state.opening || state.navigating) LinearProgressIndicator(Modifier.fillMaxWidth())
                if (state.overview == null) {
                    Text(stringResource(R.string.library), style = MaterialTheme.typography.headlineMedium)
                    Button({ authentication = Authentication.Create }, Modifier.testTag("create-database")) { Text(stringResource(R.string.create_database)) }
                    OutlinedButton(attachments.importDatabase) { Text(stringResource(R.string.open_database)) }
                    OutlinedButton({ join = true }) { Text(stringResource(R.string.receive_database)) }
                    state.catalog.forEach { item ->
                        OutlinedButton({ authentication = Authentication.Open(item) }, Modifier.fillMaxWidth().testTag("catalog-${item.database}")) { Text(item.database.take(12)) }
                    }
                    state.pending.forEach { request ->
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(stringResource(R.string.waiting_approval), Modifier.weight(1f))
                            TextButton({ state.resumeJoin(request.request) }) { Text(stringResource(R.string.continue_action)) }
                        }
                    }
                } else if (state.inspectedEntry != null) {
                    EntryDetails(state, attachments, copy)
                } else if (state.entry != null) {
                    EntryForm(state, attachments, copy)
                } else if (state.metadata != null) {
                    MetadataForm(state)
                } else {
                    val overview = state.overview ?: return@Column
                    OutlinedTextField(state.query, state::search, Modifier.fillMaxWidth().testTag("entry-search"), singleLine = true, label = { Text(stringResource(R.string.search)) })
                    Text(if (state.ungrouped) stringResource(R.string.ungrouped) else overview.groups.firstOrNull { it.id == state.selectedGroup }?.name ?: stringResource(R.string.all_entries))
                    if (!overview.writable) Text(stringResource(R.string.read_only))
                    Button({ state.beginEntry() }, enabled = overview.writable, modifier = Modifier.testTag("add-entry")) { Text(stringResource(R.string.add_entry)) }
                    val rows = overview.entries.filter { if (state.ungrouped) it.group == null else state.selectedGroup == null || it.group == state.selectedGroup }
                    if (rows.isEmpty()) Text(stringResource(R.string.no_entries))
                    rows.forEach { row ->
                        TextButton({ state.selectEntry(row.id) }, Modifier.fillMaxWidth().testTag("entry-${row.id}")) {
                            Column(Modifier.fillMaxWidth()) { Text(row.title); row.username?.let { Text(it, style = MaterialTheme.typography.bodySmall) } }
                        }
                    }
                }
                state.undo?.let { removed ->
                    Snackbar(action = { TextButton(state::undoTrash) { Text(stringResource(R.string.undo)) } }) { Text(stringResource(R.string.moved_to_trash)) }
                }
                Spacer(Modifier.height(12.dp))
            }
            }
        }
    }
    authentication?.let { auth ->
        AuthenticationDialog(state, auth, { if (auth is Authentication.Import) state.selectDatabaseImport(null); authentication = null })
    }
    if (join) InputDialog(R.string.receive_database, R.string.invitation_code, { join = false }) { code -> state.join(code); join = false }
    if (forms) AlertDialog(onDismissRequest = { forms = false }, title = { Text(stringResource(R.string.local_forms)) }, text = {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            state.forms.forEach { form ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    TextButton({ state.resumeForm(form); forms = false }, Modifier.weight(1f)) { Text("${stringResource(formText(form.kind))} · ${form.draft.take(8)}") }
                    TextButton({ state.deleteForm(form) }) { Text(stringResource(R.string.discard)) }
                }
            }
        }
    }, confirmButton = { TextButton({ forms = false }) { Text(stringResource(R.string.close)) } })
    state.history?.let { HistoryDialog(state, attachments, it, copy) }
    state.exchange?.let { ExchangeDialog(state, it, copy) }
    state.policy?.let { PolicyDialog(state, it) }
}

@Composable
private fun EntryDetails(state: DocumentState, attachments: AttachmentActions, copy: (String) -> Unit) {
    val entry = state.inspectedEntry ?: return
    Row(Modifier.horizontalScroll(rememberScrollState())) {
        TextButton({ state.chooseGroup(state.selectedGroup, state.ungrouped) }) { Text(stringResource(R.string.back)) }
        TextButton({ state.beginEntry(entry.id) }, enabled = state.overview?.writable == true, modifier = Modifier.testTag("edit-entry")) { Text(stringResource(R.string.edit)) }
        TextButton({ state.showHistory(FormKind.ENTRY, entry.id) }) { Text(stringResource(R.string.history)) }
        TextButton({ state.trash(FormKind.ENTRY, entry.id, entry.group) }, enabled = state.overview?.writable == true) { Text(stringResource(R.string.trash)) }
    }
    if (state.entryTab == 2) { AppearanceSummary(state.appearance); return }
    if (state.entryTab == 3) { PropertiesContent(state.properties); return }
    if (state.entryTab == 4) { HistoryContent(state, entry.id); return }
    if (state.entryTab == 1) {
        Text(stringResource(R.string.attributes), style = MaterialTheme.typography.titleMedium)
        state.inspectedAttributes.forEach { attribute ->
            Text(attribute.name); Text(if (attribute.protected) stringResource(R.string.protected_value) else attribute.value.orEmpty())
            if (attribute.protected) TextButton({ state.reveal(attribute.id) }) { Text(stringResource(R.string.reveal)) }
            else attribute.value?.let { value -> TextButton({ copy(value) }) { Text(stringResource(R.string.copy)) } }
        }
        if (state.revealed.isNotEmpty()) SecretValue(state.revealed, state::hideSecrets, copy)
        Attachments(state, attachments, state.attachments, entry.id)
        return
    }
    listOf(R.string.title to entry.title, R.string.username to entry.username, R.string.url to entry.url, R.string.notes to entry.notes).forEach { (label, value) ->
        Text(stringResource(label), style = MaterialTheme.typography.labelLarge)
        Text(value ?: "—")
        if (value != null) TextButton({ copy(value) }) { Text(stringResource(R.string.copy)) }
    }
    Text(stringResource(R.string.tags)); Text(state.properties?.tags?.joinToString("\n").orEmpty().ifEmpty { "—" })
    Text(stringResource(R.string.expiry)); Text(state.properties?.expiresAt?.let(::displayTime) ?: "—")
    Text(stringResource(R.string.password), style = MaterialTheme.typography.labelLarge)
    if (entry.hasPassword == true) TextButton({ state.reveal() }) { Text(stringResource(R.string.reveal)) } else Text("—")
    if (state.revealed.isNotEmpty()) SecretValue(state.revealed, state::hideSecrets, copy)
}

@Composable
private fun AuthenticationDialog(state: DocumentState, authentication: Authentication, dismiss: () -> Unit) {
    var name by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var repeat by remember { mutableStateOf("") }
    val operation = remember { newOperation() }
    LaunchedEffect(state.overview, state.opening) { if (state.overview != null && !state.opening) dismiss() }
    AlertDialog(onDismissRequest = { if (!state.opening) dismiss() }, title = { Text(stringResource(if (authentication == Authentication.Create) R.string.create_database else R.string.unlock)) }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (authentication == Authentication.Create) {
                OutlinedTextField(name, { name = it }, Modifier.testTag("database-name"), label = { Text(stringResource(R.string.name)) })
                OutlinedTextField(description, { description = it }, label = { Text(stringResource(R.string.description)) })
            }
            OutlinedTextField(password, { password = it }, Modifier.testTag("master-password"), label = { Text(stringResource(R.string.master_password)) }, visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), singleLine = true)
            if (authentication == Authentication.Create) OutlinedTextField(repeat, { repeat = it }, Modifier.testTag("repeat-master-password"), label = { Text(stringResource(R.string.repeat_master_password)) }, visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), singleLine = true)
            if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
        }
    }, confirmButton = { TextButton({
        when (authentication) {
            Authentication.Create -> state.create(name, description.ifEmpty { null }, password, operation)
            is Authentication.Open -> state.unlock(authentication.copy, password)
            is Authentication.Import -> state.import(authentication.uri, password)
        }
    }, enabled = !state.opening && (authentication != Authentication.Create || password == repeat), modifier = Modifier.testTag("authenticate")) { Text(stringResource(if (authentication == Authentication.Create) R.string.create else R.string.unlock)) } }, dismissButton = { TextButton(dismiss, enabled = !state.opening) { Text(stringResource(R.string.cancel)) } })
}

@Composable
private fun EntryForm(state: DocumentState, attachments: AttachmentActions, copy: (String) -> Unit) {
    val input = state.entry ?: return
    key(input.source.form.draft) {
        var attribute by remember { mutableStateOf<AttributeRow?>(null) }
        var addAttribute by remember { mutableStateOf(false) }
        Row(Modifier.horizontalScroll(rememberScrollState())) {
            TextButton({ state.chooseGroup(state.selectedGroup, state.ungrouped) }) { Text(stringResource(R.string.back)) }
            input.source.entry?.let { id ->
                TextButton({ state.showHistory(FormKind.ENTRY, id) }) { Text(stringResource(R.string.history)) }
                TextButton({ state.trash(FormKind.ENTRY, id, input.source.group) }) { Text(stringResource(R.string.trash)) }
            }
        }
        if (state.entryTab == 0) {
            Field(input.title, R.string.title, "entry-title", !state.navigating && state.overview?.writable == true) { state.edit(EntryTextField.TITLE, it) }
            Field(input.username, R.string.username, "entry-username", !state.navigating && state.overview?.writable == true) { state.edit(EntryTextField.USERNAME, it) }
            OutlinedTextField(input.password, { state.edit(EntryTextField.PASSWORD, it) }, Modifier.fillMaxWidth().testTag("entry-password"), enabled = !state.navigating && state.overview?.writable == true,
                label = { Text(stringResource(R.string.password)) }, placeholder = { if (input.source.hasPassword) Text(stringResource(R.string.password_retained)) }, visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password))
            Row {
                if (input.source.entry != null && input.source.hasPassword) TextButton({ state.reveal() }) { Text(stringResource(R.string.reveal)) }
                TextButton({ state.edit(EntryTextField.PASSWORD, "", true) }, enabled = state.overview?.writable == true) { Text(stringResource(R.string.clear)) }
                TextButton({ state.saveNow(); state.generator = true }) { Text(stringResource(R.string.generator)) }
            }
            if (state.revealed.isNotEmpty()) SecretValue(state.revealed, state::hideSecrets, copy)
            Field(input.url, R.string.url, "entry-url", !state.navigating && state.overview?.writable == true) { state.edit(EntryTextField.URL, it) }
            Field(input.tags, R.string.tags, "entry-tags", !state.navigating && state.overview?.writable == true, false) { state.editTags(it) }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.expiry), Modifier.weight(1f))
                Switch(input.expiryEnabled, state::enableExpiry, enabled = !state.navigating && state.overview?.writable == true)
            }
            if (input.expiryEnabled) Field(input.expiry, R.string.expiry, "entry-expiry", !state.navigating && state.overview?.writable == true) { state.editExpiry(it) }
            Field(input.notes, R.string.notes, "entry-notes", !state.navigating && state.overview?.writable == true, false) { state.edit(EntryTextField.NOTES, it) }
        } else if (state.entryTab == 1) {
            input.source.attributes.forEach { row ->
                key(row.id ?: row.name) { EditableAttribute(state, row) { attribute = it } }
            }
            TextButton({ addAttribute = true }, enabled = state.overview?.writable == true) { Text(stringResource(R.string.add_attribute)) }
            if (state.revealed.isNotEmpty()) SecretValue(state.revealed, state::hideSecrets, copy)
            Attachments(state, attachments, state.attachments, input.source.entry, editing = state.overview?.writable == true)
        } else if (state.entryTab == 2) {
            IconPicker(state.appearance?.lucide, state::setIcon)
            ColorPicker(R.string.foreground_color, state.appearance?.foreground) { value -> state.setColors(value?.let(ColorEdit::Set) ?: ColorEdit.Clear, ColorEdit.Keep) }
            ColorPicker(R.string.background_color, state.appearance?.background) { value -> state.setColors(ColorEdit.Keep, value?.let(ColorEdit::Set) ?: ColorEdit.Clear) }
        } else if (state.entryTab == 3) {
            PropertiesContent(state.properties)
        } else {
            HistoryContent(state, input.source.entry)
        }
        if (addAttribute) AttributeDialog(null, { addAttribute = false }) { id, name, value, protected ->
            state.editAttribute(id, name, value, protected); addAttribute = false
        }
        attribute?.let { row ->
            var name by remember(row.id) { mutableStateOf(row.name) }
            AlertDialog(onDismissRequest = { attribute = null }, title = { Text(stringResource(R.string.rename)) },
                text = { TextField(name, { name = it }, label = { Text(stringResource(R.string.name)) }) },
                confirmButton = { TextButton({ state.editAttribute(row.id, name, null, row.protected); attribute = null }) { Text(stringResource(R.string.apply)) } },
                dismissButton = { TextButton({ attribute = null }) { Text(stringResource(R.string.cancel)) } })
        }
    }
}

/** Existing values are addressed edits; only constructing/renaming a row has an Apply command. */
@Composable
private fun EditableAttribute(state: DocumentState, row: AttributeRow, rename: (AttributeRow) -> Unit) {
    var value by remember { mutableStateOf(row.value.orEmpty()) }
    var protected by remember { mutableStateOf(row.protected) }
    var typed by remember { mutableStateOf(false) }
    val enabled = !state.navigating && state.overview?.writable == true
    LaunchedEffect(row.value, protected) {
        // Removing protection is an explicit user action; its acknowledged unmasked
        // value can fill untouched input, while later typing retains focus and text.
        if (!typed && !protected && row.value != null) value = row.value.orEmpty()
    }
    Column(Modifier.fillMaxWidth()) {
        Text(row.name)
        TextField(value, { value = it; typed = true; state.editAttribute(row.id, row.name, it, protected) },
            modifier = Modifier.fillMaxWidth().testTag("attribute-value-${row.id}"), enabled = enabled,
            label = { Text(stringResource(R.string.value)) },
            placeholder = { if (protected && row.value == null) Text(stringResource(R.string.protected_value)) },
            visualTransformation = if (protected) PasswordVisualTransformation() else VisualTransformation.None)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.protected_value), Modifier.weight(1f))
            Checkbox(protected, { protected = it; state.editAttribute(row.id, row.name, null, it) },
                enabled = enabled, modifier = Modifier.testTag("attribute-protected-${row.id}"))
            TextButton({ rename(row.copy(protected = protected)) }, enabled = enabled) { Text(stringResource(R.string.rename)) }
            if (protected && row.id != null) TextButton({ state.reveal(row.id) }) { Text(stringResource(R.string.reveal)) }
            TextButton({ state.editAttribute(row.id, row.name, null, protected, true) }, enabled = enabled) { Text(stringResource(R.string.remove)) }
        }
    }
}

@Composable
private fun MetadataForm(state: DocumentState) {
    val input = state.metadata ?: return
    Row(Modifier.horizontalScroll(rememberScrollState())) {
        TextButton({ state.chooseGroup(state.selectedGroup, state.ungrouped) }) { Text(stringResource(R.string.back)) }
        if (input.source.form.kind == FormKind.GROUP || input.source.form.kind == FormKind.DATABASE) {
            TextButton({ state.showHistory(input.source.form.kind, input.source.form.target) }) { Text(stringResource(R.string.history)) }
        }
        if (input.source.form.kind == FormKind.GROUP && input.source.form.target != null) TextButton({ state.trash(FormKind.GROUP, input.source.form.target!!, null) }) { Text(stringResource(R.string.trash)) }
        if (input.source.form.kind == FormKind.DATABASE && state.overview?.managing == true) TextButton(state::showPolicy) { Text(stringResource(R.string.policy)) }
    }
    Field(input.name, R.string.name, "metadata-name", !state.navigating) { state.editMetadata(name = it) }
    Field(input.description, R.string.description, "metadata-description", !state.navigating, false) { state.editMetadata(description = it) }
    TextButton({ state.editMetadata(description = "", clearDescription = true) }) { Text(stringResource(R.string.clear_description)) }
    if (input.source.form.kind == FormKind.GROUP || input.source.form.kind == FormKind.NEW_GROUP) IconPicker(null, state::setGroupIcon)
}

@Composable
private fun EntryTabs(state: DocumentState) {
    val labels = listOf(R.string.details, R.string.additional, R.string.appearance, R.string.properties, R.string.history)
    ScrollableTabRow(selectedTabIndex = state.entryTab, edgePadding = 0.dp) {
        labels.forEachIndexed { index, label -> Tab(state.entryTab == index, { state.selectTab(index) }, Modifier.testTag("entry-tab-$index"), text = { Text(stringResource(label)) }) }
    }
}
@Composable
private fun AppearanceSummary(value: AppearanceView?) {
    Text(stringResource(R.string.icon)); Text(value?.lucide ?: value?.image?.take(12) ?: stringResource(R.string.default_icon))
    Text(stringResource(R.string.foreground_color)); Text(value?.foreground?.toString(16)?.padStart(8, '0') ?: stringResource(R.string.follow_theme))
    Text(stringResource(R.string.background_color)); Text(value?.background?.toString(16)?.padStart(8, '0') ?: stringResource(R.string.follow_theme))
}
@Composable
private fun PropertiesContent(value: EntryProperties?) {
    if (value == null) { Text(stringResource(R.string.no_saved_properties)); return }
    Text(stringResource(R.string.created_at)); Text(displayTime(value.createdAt))
    Text(stringResource(R.string.modified_at)); Text(displayTime(value.modifiedAt))
    Text(stringResource(R.string.object_id)); Text(value.entry)
    Text(stringResource(R.string.expiry)); Text(value.expiresAt?.let(::displayTime) ?: "—")
    Text(stringResource(R.string.tags)); Text(value.tags.joinToString("\n").ifEmpty { "—" })
    AppearanceSummary(value.appearance)
}
@Composable
private fun HistoryContent(state: DocumentState, entry: String?) {
    if (state.entryHistory.isEmpty()) Text(stringResource(R.string.no_saved_versions))
    state.entryHistory.forEach { row ->
        TextButton({ if (entry != null) state.showHistory(FormKind.ENTRY, entry) }) { Column { Text(row.name); Text(displayTime(row.savedAt)) } }
    }
}
@Composable
private fun IconPicker(current: String?, choose: (String?) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val keys = remember { bundledIconKeys() }
    Box {
        TextButton({ open = true }, Modifier.testTag("icon-picker")) { Text("${stringResource(R.string.icon)}: ${current ?: stringResource(R.string.choose_icon)}") }
        DropdownMenu(open, { open = false }) {
            DropdownMenuItem({ Text(stringResource(R.string.default_icon)) }, { choose(null); open = false })
            keys.forEach { key -> DropdownMenuItem({ Text(key) }, { choose(key); open = false }, Modifier.testTag("icon-key-$key")) }
        }
    }
}
@Composable
private fun ColorPicker(label: Int, value: UInt?, choose: (UInt?) -> Unit) {
    Text(stringResource(label), style = MaterialTheme.typography.labelLarge)
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(stringResource(R.string.follow_theme), Modifier.weight(1f))
        Switch(value == null, { follows -> choose(if (follows) null else 0x000000ffu) })
    }
    if (value != null) {
        val argb = ((value and 0xffu) shl 24) or (value shr 8)
        Surface(color = androidx.compose.ui.graphics.Color(argb.toInt()), modifier = Modifier.fillMaxWidth().height(24.dp)) {}
        Text("#${value.toString(16).padStart(8, '0')}")
        listOf("R", "G", "B", "A").forEachIndexed { index, channel ->
            val shift = (3 - index) * 8
            val component = ((value shr shift) and 0xffu).toInt()
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(channel)
                Slider(component.toFloat(), { position ->
                    val replacement = position.roundToInt().coerceIn(0, 255).toUInt()
                    choose((value and (0xffu shl shift).inv()) or (replacement shl shift))
                }, valueRange = 0f..255f, steps = 254, modifier = Modifier.weight(1f))
                Text(component.toString())
            }
        }
    }
}
private fun displayTime(milliseconds: Long): String = java.time.format.DateTimeFormatter.ofLocalizedDateTime(java.time.format.FormatStyle.MEDIUM)
    .withLocale(java.util.Locale.getDefault()).withZone(java.time.ZoneId.systemDefault()).format(java.time.Instant.ofEpochMilli(milliseconds))

@Composable
private fun Field(value: String, label: Int, tag: String, enabled: Boolean, single: Boolean = true, update: (String) -> Unit) {
    Text(stringResource(label), style = MaterialTheme.typography.labelLarge)
    TextField(value, update, Modifier.fillMaxWidth().testTag(tag), enabled = enabled, singleLine = single,
        colors = TextFieldDefaults.colors(unfocusedIndicatorColor = androidx.compose.ui.graphics.Color.Transparent))
}
@Composable
private fun SecretValue(value: String, hide: () -> Unit, copy: (String) -> Unit) {
    val view = androidx.compose.ui.platform.LocalView.current
    val context = androidx.compose.ui.platform.LocalContext.current
    val app = context.applicationContext as TaypeerApplication
    val latestHide by rememberUpdatedState(hide)
    DisposableEffect(view) {
        val observer = view.viewTreeObserver
        val listener = ViewTreeObserver.OnWindowFocusChangeListener { focused ->
            try { app.clipboard.focusChanged(focused) } catch (_: Exception) { /* Copy reports failure separately. */ }
            if (!focused) latestHide()
        }
        observer.addOnWindowFocusChangeListener(listener)
        try { app.clipboard.focusChanged(view.hasWindowFocus()) } catch (_: Exception) { /* Remain unavailable. */ }
        onDispose {
            if (observer.isAlive) observer.removeOnWindowFocusChangeListener(listener)
            try { app.clipboard.focusChanged(context.activity()?.hasWindowFocus() == true) } catch (_: Exception) { /* Remain unavailable. */ }
        }
    }
    Text(value, Modifier.testTag("revealed-secret"))
    Row { TextButton(hide) { Text(stringResource(R.string.hide)) }; TextButton({ copy(value) }) { Text(stringResource(R.string.copy)) } }
}
private fun Context.activity(): android.app.Activity? = when (this) {
    is android.app.Activity -> this
    is ContextWrapper -> baseContext.activity()
    else -> null
}
@Composable
private fun AttributeDialog(row: AttributeRow?, dismiss: () -> Unit, confirm: (String?, String, String?, Boolean) -> Unit) {
    var name by remember { mutableStateOf(row?.name.orEmpty()) }
    var value by remember { mutableStateOf(row?.value.orEmpty()) }
    var changed by remember { mutableStateOf(row == null || row.protected.not()) }
    var protected by remember { mutableStateOf(row?.protected ?: true) }
    AlertDialog(onDismissRequest = dismiss, title = { Text(stringResource(R.string.attribute)) }, text = {
        Column {
            OutlinedTextField(name, { name = it }, label = { Text(stringResource(R.string.name)) }, modifier = Modifier.testTag("new-attribute-name"))
            OutlinedTextField(value, { value = it; changed = true }, label = { Text(stringResource(R.string.value)) }, visualTransformation = PasswordVisualTransformation(), modifier = Modifier.testTag("new-attribute-value"))
            Row(verticalAlignment = Alignment.CenterVertically) { Text(stringResource(R.string.protected_value), Modifier.weight(1f)); Switch(protected, { protected = it }) }
        }
    }, confirmButton = { TextButton({ confirm(row?.id, name, if (changed) value else null, protected) }) { Text(stringResource(R.string.apply)) } }, dismissButton = { TextButton(dismiss) { Text(stringResource(R.string.cancel)) } })
}
@Composable
private fun InputDialog(title: Int, label: Int, dismiss: () -> Unit, secret: Boolean = false, confirm: (String) -> Unit) {
    var value by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = dismiss, title = { Text(stringResource(title)) }, text = {
        Column { OutlinedTextField(value, { value = it }, label = { Text(stringResource(label)) }, visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None,
            keyboardOptions = KeyboardOptions(keyboardType = if (secret) KeyboardType.Password else KeyboardType.Text))
            if (secret) Text(stringResource(R.string.rotation_warning))
        }
    },
        confirmButton = { TextButton({ confirm(value) }) { Text(stringResource(R.string.continue_action)) } }, dismissButton = { TextButton(dismiss) { Text(stringResource(R.string.cancel)) } })
}

@Composable
private fun HistoryDialog(state: DocumentState, attachments: AttachmentActions, selection: HistorySelection, copy: (String) -> Unit) {
    var purge by remember { mutableStateOf(false) }
    var restore by remember { mutableStateOf<Pair<String, String>?>(null) }
    var reviewed by remember(selection.rows) { mutableStateOf<List<String>>(emptyList()) }
    val operation = remember(reviewed) { newOperation() }
    AlertDialog(onDismissRequest = state::dismissHistory, title = { Text(stringResource(R.string.history)) }, text = {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            Text(stringResource(R.string.original_versions))
            selection.rows.forEach { row ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(row.id in reviewed, { checked -> reviewed = if (checked) reviewed + row.id else reviewed - row.id })
                    TextButton({ state.selectRevision(row) }) { Column { Text(row.name); Text(displayTime(row.savedAt), style = MaterialTheme.typography.bodySmall); row.description?.let { Text(it) } } }
                }
            }
            state.historicalEntry?.let { entry ->
                Text(entry.title); entry.username?.let { Text(it) }; entry.notes?.let { Text(it) }
                if (entry.hasPassword == true) TextButton({ state.reveal() }) { Text(stringResource(R.string.reveal)) }
                state.historicalAttributes.forEach { attribute ->
                    Text(attribute.name); Text(if (attribute.protected) stringResource(R.string.protected_value) else attribute.value.orEmpty())
                    if (attribute.protected) TextButton({ state.reveal(attribute.id) }) { Text(stringResource(R.string.reveal)) }
                    else attribute.value?.let { value -> TextButton({ copy(value) }) { Text(stringResource(R.string.copy)) } }
                }
                val revision = state.historicalRevision
                if (revision != null && state.overview?.writable == true) TextButton({ restore = entry.id to revision }) { Text(stringResource(R.string.restore_version)) }
                Attachments(state, attachments, state.historicalAttachments, entry.id, revision)
            }
            if (state.revealed.isNotEmpty()) SecretValue(state.revealed, state::hideSecrets, copy)
        }
    }, confirmButton = { TextButton(state::dismissHistory) { Text(stringResource(R.string.close)) } }, dismissButton = { TextButton({ purge = true }, enabled = reviewed.isNotEmpty() && state.overview?.writable == true) { Text(stringResource(R.string.purge_selected)) } })
    if (purge) AlertDialog(onDismissRequest = { purge = false }, title = { Text(stringResource(R.string.purge_history)) }, text = { Text(stringResource(R.string.purge_history_warning)) },
        confirmButton = { TextButton({ state.purgeHistory(selection, reviewed, operation); purge = false }) { Text(stringResource(R.string.delete_permanently)) } }, dismissButton = { TextButton({ purge = false }) { Text(stringResource(R.string.cancel)) } })
    restore?.let { reviewedVersion ->
        val restoreOperation = remember(reviewedVersion) { newOperation() }
        AlertDialog(onDismissRequest = { restore = null }, title = { Text(stringResource(R.string.restore_version)) }, text = { Text(stringResource(R.string.restore_version_warning)) },
            confirmButton = { TextButton({ state.restoreRevision(reviewedVersion.first, reviewedVersion.second, restoreOperation); restore = null }) { Text(stringResource(R.string.continue_action)) } },
            dismissButton = { TextButton({ restore = null }) { Text(stringResource(R.string.cancel)) } })
    }
}

@Composable
private fun ExchangeDialog(state: DocumentState, exchange: ExchangeView, copy: (String) -> Unit) {
    var approving by remember { mutableStateOf<String?>(null) }
    var rotating by remember { mutableStateOf<String?>(null) }
    var rotation by remember { mutableStateOf(false) }
    val context = androidx.compose.ui.platform.LocalContext.current
    AlertDialog(onDismissRequest = state::dismissExchange, title = { Text(stringResource(R.string.sync)) }, text = {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            Text(stringResource(R.string.encrypted_exchange_note))
            TextButton(state::exchange) { Text(stringResource(R.string.refresh)) }
            TextButton({ ExchangeScheduler.stopForeground(context) }) { Text(stringResource(R.string.stop_exchange)) }
            exchange.devices.forEach { device ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(device.id.take(12), Modifier.weight(1f))
                    if (device.manager) Text(stringResource(R.string.managing_device))
                    if (state.overview?.managing == true && !device.local) TextButton({ rotating = device.id; rotation = true }) { Text(stringResource(R.string.revoke)) }
                }
            }
            exchange.invitations.filter { it.phase == InvitationPhase.REQUESTED }.forEach { request ->
                Text(request.recipient?.take(12).orEmpty())
                Row { TextButton({ approving = request.request }) { Text(stringResource(R.string.approve)) }; TextButton({ state.reject(request.request) }) { Text(stringResource(R.string.reject)) } }
            }
            if (state.overview?.managing == true) {
                TextButton(state::invite) { Text(stringResource(R.string.share_database)) }
                TextButton({ rotation = true }) { Text(stringResource(R.string.change_password)) }
            }
            if (state.invitation.isNotEmpty()) { Text(stringResource(R.string.invitation_expires)); SecretValue(state.invitation, state::hideSecrets, copy) }
        }
    }, confirmButton = { TextButton(state::dismissExchange) { Text(stringResource(R.string.close)) } })
    approving?.let { request -> AlertDialog(onDismissRequest = { approving = null }, title = { Text(stringResource(R.string.approve_device)) }, text = { Text(stringResource(R.string.approve_device_warning)) },
        confirmButton = { TextButton({ state.approve(request); approving = null }) { Text(stringResource(R.string.approve)) } }, dismissButton = { TextButton({ approving = null }) { Text(stringResource(R.string.cancel)) } }) }
    if (rotation) {
        val operation = remember { newOperation() }
        InputDialog(R.string.change_password, R.string.new_master_password, { rotation = false; rotating = null }, secret = true) { password -> state.rotate(password, rotating, operation); rotation = false; rotating = null }
    }
}
@Composable
private fun PolicyDialog(state: DocumentState, value: PolicyView) {
    var attachment by remember { mutableStateOf(value.attachmentBytes.toString()) }
    var total by remember { mutableStateOf(value.totalAttachmentBytes.toString()) }
    var target by remember { mutableStateOf(value.kdfTargetMs.toString()) }
    var password by remember { mutableStateOf("") }
    val operation = remember(attachment, total, target, password) { newOperation() }
    AlertDialog(onDismissRequest = state::dismissPolicy, title = { Text(stringResource(R.string.policy)) }, text = {
        Column {
            OutlinedTextField(attachment, { attachment = it }, label = { Text(stringResource(R.string.attachment_limit)) })
            OutlinedTextField(total, { total = it }, label = { Text(stringResource(R.string.total_attachment_limit)) })
            OutlinedTextField(target, { target = it }, label = { Text(stringResource(R.string.kdf_target)) })
            OutlinedTextField(password, { password = it }, label = { Text(stringResource(R.string.master_password)) }, visualTransformation = PasswordVisualTransformation())
            Text(stringResource(R.string.policy_confirmation))
            if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error)
        }
    }, confirmButton = { TextButton({
        val one = attachment.toULongOrNull(); val all = total.toULongOrNull(); val ms = target.toUIntOrNull()
        if (one != null && all != null && ms != null) state.setPolicy(PolicyView(one, all, ms), password.ifEmpty { null }, operation)
        else state.reportFailure()
    }) { Text(stringResource(R.string.apply)) } }, dismissButton = { TextButton(state::dismissPolicy) { Text(stringResource(R.string.cancel)) } })
}
private fun saveText(label: SaveLabel) = when (label) {
    SaveLabel.CLEAN -> R.string.save_clean
    SaveLabel.DIRTY -> R.string.save_dirty
    SaveLabel.SAVING -> R.string.save_running
    SaveLabel.SAVED -> R.string.save_saved
    SaveLabel.LOCAL -> R.string.save_local
    SaveLabel.FAILED -> R.string.save_failed
}
private fun formText(kind: FormKind) = when (kind) {
    FormKind.ENTRY -> R.string.entry
    FormKind.NEW_ENTRY -> R.string.new_entry
    FormKind.GROUP -> R.string.group
    FormKind.NEW_GROUP -> R.string.new_group
    FormKind.DATABASE -> R.string.database_info
}
