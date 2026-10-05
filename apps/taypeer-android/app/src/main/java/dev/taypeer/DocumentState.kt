package dev.taypeer

import android.app.Application
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import dev.taypeer.platform.ExchangeScheduler
import dev.taypeer.platform.SelectedDescriptors
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import java.util.UUID

enum class SaveLabel { CLEAN, DIRTY, SAVING, SAVED, LOCAL, FAILED }
data class EntryInput(val source: EntryEditor, val title: String = source.title,
    val username: String = source.username.orEmpty(), val password: String = "",
    val url: String = source.url.orEmpty(), val notes: String = source.notes.orEmpty(),
    val expiry: String = source.expiryInput ?: source.expiresAt?.toString().orEmpty(), val expiryEnabled: Boolean = source.expiryInput != null || source.expiresAt != null,
    val tags: String = source.tags.joinToString("\n"))
data class MetadataInput(val source: MetadataEditor, val name: String = source.name,
    val description: String = source.description.orEmpty())
data class HistorySelection(val kind: FormKind, val target: String?, val rows: List<HistoryRow>)
data class UndoTarget(val kind: FormKind, val target: String, val group: String?)
/** Opaque, public SAF intent only. It survives background without retaining a session or content. */
sealed interface AttachmentSelection {
    val database: String
    val operation: String
    val uri: Uri?
    data class Import(override val database: String, val draft: String, val replacement: String?,
        override val operation: String, override val uri: Uri? = null) : AttachmentSelection
    data class Export(override val database: String, val entry: String, val revision: String?, val blob: String,
        override val operation: String, override val uri: Uri? = null, val draft: String? = null) : AttachmentSelection
}

/** Only in-memory presentation. Rust owns forms, validation, receipts and encrypted persistence. */
class DocumentState(application: Application) : AndroidViewModel(application) {
    private val app = application as TaypeerApplication
    private val manager = viewModelScope.async(Dispatchers.IO) {
        DocumentManager(app, app.host.await()).also { owner ->
            owner.clearSensitiveUi = { clearSensitive() }
            ExchangeScheduler.schedule(app)
        }
    }
    private val commands = Channel<suspend () -> Unit>(Channel.UNLIMITED)
    private var session: DocumentSession? = null
    private var epoch = 0L
    private var inputRevision = 0L
    private var appliedInputRevision = 0L
    private var secretEpoch = 0L
    private val cleanup = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var saveJob: Job? = null
    private val receipts = mutableMapOf<Pair<String, ULong>, String>()
    private val retryEdits = linkedMapOf<String, suspend (DocumentSession) -> Unit>()
    private val pendingUnpins = linkedSetOf<Pair<String, String>>()
    var catalog by mutableStateOf<List<WorkingCopyView>>(emptyList()); private set
    var overview by mutableStateOf<DocumentOverview?>(null); private set
    var entry by mutableStateOf<EntryInput?>(null); private set
    var inspectedEntry by mutableStateOf<EntryRow?>(null); private set
    var metadata by mutableStateOf<MetadataInput?>(null); private set
    var forms by mutableStateOf<List<LocalForm>>(emptyList()); private set
    var activeForm by mutableStateOf<LocalForm?>(null); private set
    var history by mutableStateOf<HistorySelection?>(null); private set
    var historicalEntry by mutableStateOf<EntryRow?>(null); private set
    var historicalRevision by mutableStateOf<String?>(null); private set
    var revealed by mutableStateOf(""); private set
    var invitation by mutableStateOf(""); private set
    var exchange by mutableStateOf<ExchangeView?>(null); private set
    var pending by mutableStateOf<List<EnrollmentRow>>(emptyList()); private set
    var policy by mutableStateOf<PolicyView?>(null); private set
    var appearance by mutableStateOf<AppearanceView?>(null); private set
    var properties by mutableStateOf<EntryProperties?>(null); private set
    var entryHistory by mutableStateOf<List<HistoryRow>>(emptyList()); private set
    var inspectedAttributes by mutableStateOf<List<AttributeRow>>(emptyList()); private set
    var attachments by mutableStateOf<List<AttachmentRow>>(emptyList()); private set
    var historicalAttachments by mutableStateOf<List<AttachmentRow>>(emptyList()); private set
    var historicalAttributes by mutableStateOf<List<AttributeRow>>(emptyList()); private set
    var entryTab by mutableIntStateOf(0)
    var attachmentSelection by mutableStateOf<AttachmentSelection?>(null); private set
    var databaseImport by mutableStateOf<Uri?>(null); private set
    fun selectDatabaseImport(uri: Uri?) { databaseImport = uri }
    var undo by mutableStateOf<UndoTarget?>(null); private set
    var selectedGroup by mutableStateOf<String?>(null); private set
    var ungrouped by mutableStateOf(false); private set
    var query by mutableStateOf(""); private set
    var saveLabel by mutableStateOf(SaveLabel.CLEAN); private set
    var error by mutableStateOf(false); private set
    var opening by mutableStateOf(false); private set
    var navigating by mutableStateOf(false); private set
    var generator by mutableStateOf(false)
    var secrecyEpoch by mutableStateOf(0L); private set

    private data class Binding(val session: DocumentSession, val epoch: Long, val database: String, val generation: ULong)
    init {
        viewModelScope.launch {
            for (command in commands) {
                try { command() }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { error = true; saveLabel = SaveLabel.FAILED; navigating = false; opening = false }
            }
        }
        reloadCatalog()
        viewModelScope.launch {
            while (isActive) {
                delay(100)
                val current = session
                if (current != null && current.access() != SessionAccess.OPEN) clearSensitive()
            }
        }
    }
    private fun binding(): Binding? = session?.let { current ->
        overview?.let { Binding(current, epoch, it.database, current.generation()) }
    }
    private fun current(binding: Binding): Boolean = session === binding.session && epoch == binding.epoch &&
        overview?.database == binding.database && binding.session.generation() == binding.generation &&
        binding.session.access() == SessionAccess.OPEN
    private suspend fun <T> io(action: () -> T): T = withContext(Dispatchers.IO) { action() }
    private fun enqueue(action: suspend () -> Unit) { commands.trySend(action) }
    private fun bound(action: suspend (Binding) -> Unit) {
        val binding = binding() ?: return
        enqueue {
            if (current(binding)) {
                try { action(binding) }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (session === binding.session && epoch == binding.epoch && binding.session.access() != SessionAccess.OPEN) clearSensitive()
                    else if (current(binding)) { error = true; saveLabel = SaveLabel.FAILED; navigating = false }
                }
            }
        }
    }
    fun reloadCatalog() = enqueue {
        val owner = manager.await()
        val copies = io { owner.catalog() }
        val requests = io { app.host.awaitBlocking().pendingJoins() }
        catalog = copies; pending = requests
    }
    fun create(name: String, description: String?, password: String, operation: String) = open {
        it.create(name, description, operation, password)
    }
    fun unlock(copy: WorkingCopyView, password: String) = open { it.open(copy.path, password) }
    fun import(uri: Uri, password: String) = open { it.import(uri, password) }
    private fun open(action: (DocumentManager) -> DocumentSession) {
        val requestedEpoch = epoch
        val previousBinding = binding()
        saveJob?.cancel(); opening = true; navigating = true; error = false
        enqueue {
            if (epoch != requestedEpoch) return@enqueue
            val owner = manager.await()
            if (previousBinding != null) {
                if (!current(previousBinding)) return@enqueue
                flush(previousBinding)
                if (!current(previousBinding)) return@enqueue
            }
            val previous = session
            // Detach before revoking: the independent access observer must not
            // interpret this deliberate close as a background invalidation of the new open.
            session = null
            val openingEpoch = ++epoch
            clearContent()
            if (previous != null) { previous.lock(); io { owner.close(previous) } }
            val opened = io { action(owner) }
            if (epoch != openingEpoch) { opened.lock(); io { owner.close(opened) }; return@enqueue }
            val view = io { opened.overview("") }
            if (epoch != openingEpoch || opened.access() != SessionAccess.OPEN) {
                opened.lock(); io { owner.close(opened) }; return@enqueue
            }
            clearContent()
            session = opened; overview = view; opening = false; navigating = false; databaseImport = null
            val binding = binding() ?: return@enqueue
            releaseAbandonedPins(binding)
            val active = io { opened.activeForm() }
            if (active != null) { io { opened.resumeForm(active.draft) }; loadForm(binding, active) }
            refresh(binding)
            catalog = io { owner.catalog() }
        }
    }
    private suspend fun refresh(binding: Binding) {
        val expectedQuery = query
        val view = io { binding.session.overview(expectedQuery) }
        val local = io { binding.session.forms() }
        if (current(binding) && view.database == binding.database && view.generation == binding.generation) {
            if (query == expectedQuery) overview = view
            forms = local
        }
    }
    private suspend fun loadForm(binding: Binding, form: LocalForm) {
        val editor = if (form.kind == FormKind.ENTRY || form.kind == FormKind.NEW_ENTRY) io { binding.session.editor() } else null
        val descriptive = if (editor == null) io { binding.session.metadata(form.draft) } else null
        if (!current(binding)) return
        entry = editor?.let(::EntryInput); metadata = descriptive?.let(::MetadataInput); inspectedEntry = null
        activeForm = editor?.form ?: descriptive?.form
        inputRevision++; appliedInputRevision = inputRevision; revealed = ""; history = null; historicalEntry = null; historicalRevision = null
        saveLabel = if (activeForm?.dirty == true) SaveLabel.DIRTY else SaveLabel.CLEAN
        loadEntryTab(binding)
    }
    private fun navigate(action: suspend (Binding) -> Unit) {
        saveJob?.cancel(); navigating = true
        bound { binding ->
            flush(binding)
            if (current(binding)) { action(binding); navigating = false; refresh(binding) }
        }
    }
    fun chooseGroup(group: String?, onlyUngrouped: Boolean = false) = navigate { binding ->
        selectedGroup = group; ungrouped = onlyUngrouped; clearEditor()
    }
    fun beginEntry(id: String? = null) = navigate { binding ->
        val editor = io { binding.session.beginEntry(id, selectedGroup) }
        if (current(binding)) { entry = EntryInput(editor); metadata = null; inspectedEntry = null; activeForm = editor.form; inputRevision++; appliedInputRevision = inputRevision; if (id == null) entryTab = 0; loadEntryTab(binding) }
    }
    fun selectEntry(id: String) = navigate { binding ->
        val view = io { binding.session.entry(id) }
        val information = io { binding.session.entryProperties(id) }
        if (current(binding)) { clearEditor(); inspectedEntry = view; properties = information; appearance = information.appearance; loadEntryTab(binding) }
    }
    fun beginMetadata(kind: FormKind, group: String? = null) = navigate { binding ->
        val editor = io { binding.session.beginMetadata(kind, group, selectedGroup) }
        if (current(binding)) { metadata = MetadataInput(editor); entry = null; inspectedEntry = null; activeForm = editor.form; inputRevision++; appliedInputRevision = inputRevision }
    }
    fun resumeForm(form: LocalForm) = navigate { binding ->
        io { binding.session.resumeForm(form.draft) }; loadForm(binding, form)
    }
    fun deleteForm(form: LocalForm) = bound { binding ->
        io { binding.session.deleteForm(form.draft) }
        if (current(binding)) { if (activeForm?.draft == form.draft) clearEditor(); refresh(binding) }
    }
    fun search(value: String) {
        activity()
        query = value
        val expected = value
        bound { binding -> if (query == expected) refresh(binding) }
    }
    fun edit(field: EntryTextField, value: String, clear: Boolean = false) {
        val input = entry ?: return
        if (navigating) return
        entry = when (field) {
            EntryTextField.TITLE -> input.copy(title = value)
            EntryTextField.USERNAME -> input.copy(username = value)
            EntryTextField.PASSWORD -> input.copy(password = value)
            EntryTextField.URL -> input.copy(url = value)
            EntryTextField.NOTES -> input.copy(notes = value)
        }
        patch("field:$field") { active -> active.patchEntry(field, if (clear) TextEdit.Clear else TextEdit.Set(value)).form }
    }
    fun editExpiry(value: String) {
        val input = entry ?: return
        if (navigating) return
        entry = input.copy(expiry = value)
        patch("expiry") { it.patchExpiry(value).form }
    }
    fun editTags(value: String) {
        val input = entry ?: return
        if (navigating) return
        entry = input.copy(tags = value)
        patch("tags") { it.patchTags(if (value.isEmpty()) emptyList() else value.split('\n')).form }
    }
    fun setIcon(key: String?) {
        if (navigating || entry == null) return
        val operation = newOperation()
        appearance = (appearance ?: AppearanceView(null, null, null, null)).copy(lucide = key, image = null)
        patch("icon") { it.setIcon(key, operation).form }
    }
    fun setColors(foreground: ColorEdit, background: ColorEdit) {
        if (navigating || entry == null) return
        val operation = newOperation()
        val old = appearance ?: AppearanceView(null, null, null, null)
        appearance = old.copy(
            foreground = when (foreground) { ColorEdit.Clear -> null; ColorEdit.Keep -> old.foreground; is ColorEdit.Set -> foreground.rgba },
            background = when (background) { ColorEdit.Clear -> null; ColorEdit.Keep -> old.background; is ColorEdit.Set -> background.rgba })
        val address = if (foreground == ColorEdit.Keep) "colors:background" else if (background == ColorEdit.Keep) "colors:foreground" else "colors:both"
        patch(address) { it.setColors(foreground, background, operation).form }
    }
    fun setGroupIcon(key: String?) {
        val draft = activeForm?.draft ?: return
        patch("metadata:icon") { it.patchGroupIcon(draft, key?.let(TextEdit::Set) ?: TextEdit.Clear).form }
    }
    fun selectTab(tab: Int) {
        hideSecrets()
        entryTab = tab
        bound(::loadEntryTab)
    }
    private suspend fun loadEntryTab(binding: Binding) {
        val tab = entryTab
        val id = entry?.source?.entry ?: inspectedEntry?.id
        val draft = entry?.source?.form?.draft
        val requestedInput = inputRevision
        val confirmedInput = appliedInputRevision
        when (tab) {
                1 -> {
                    val rows = if (entry != null) io { binding.session.formAttachments() } else id?.let { io { binding.session.entryAttachments(it, null) } } ?: emptyList()
                    val attributes = if (entry != null) emptyList() else id?.let { io { binding.session.entryAttributes(it, null) } } ?: emptyList()
                    if (current(binding) && entryTab == tab) { attachments = rows; inspectedAttributes = attributes }
                }
                2 -> {
                    val view = if (entry != null) io { binding.session.formAppearance() } else id?.let { io { binding.session.entryProperties(it).appearance } }
                    if (current(binding) && entryTab == tab && draft == entry?.source?.form?.draft &&
                        (draft == null || (requestedInput == confirmedInput && inputRevision == requestedInput))) appearance = view
                }
                3 -> { val view = id?.let { io { binding.session.entryProperties(it) } }; if (current(binding) && entryTab == tab) properties = view }
                4 -> { val rows = id?.let { io { binding.session.history(FormKind.ENTRY, it) } } ?: emptyList(); if (current(binding) && entryTab == tab) entryHistory = rows }
        }
    }
    fun enableExpiry(enabled: Boolean) {
        val input = entry ?: return
        if (navigating) return
        entry = input.copy(expiryEnabled = enabled)
        patch("expiry") { it.patchExpiry(if (enabled) input.expiry else null).form }
    }
    fun prepareAttachment(replacement: String?, launch: (String) -> Unit) {
        val form = activeForm ?: return
        if (entry == null || navigating) return
        saveJob?.cancel(); navigating = true
        bound { binding ->
            flush(binding)
            attachmentSelection?.let(::requestUnpin)
            attachmentSelection = null
            releaseAbandonedPins(binding)
            val pinned = io { binding.session.pinActiveForm() }
            if (current(binding) && activeForm?.draft == form.draft) {
                if (pinned.draft != form.draft) { error = true; navigating = false; return@bound }
                attachmentSelection = AttachmentSelection.Import(binding.database, pinned.draft, replacement, newOperation())
                navigating = false; launch(attachmentSelection!!.operation)
            }
        }
    }
    fun prepareExport(entry: String, revision: String?, blob: String, launch: (String) -> Unit) {
        if (navigating) return
        saveJob?.cancel(); navigating = true
        bound { binding ->
            flush(binding)
            attachmentSelection?.let(::requestUnpin)
            attachmentSelection = null
            releaseAbandonedPins(binding)
            val draft = activeForm?.let { io { binding.session.pinActiveForm() }.draft }
            if (current(binding)) {
                attachmentSelection = AttachmentSelection.Export(binding.database, entry, revision, blob, newOperation(), draft = draft)
                navigating = false; launch(attachmentSelection!!.operation)
            }
        }
    }
    fun selectedAttachment(operation: String?, uri: Uri?) {
        if (operation == null || attachmentSelection?.operation != operation) return
        if (uri == null) attachmentSelection?.let(::requestUnpin)
        attachmentSelection = if (uri == null) null else when (val pending = attachmentSelection) {
            is AttachmentSelection.Import -> pending.copy(uri = uri)
            is AttachmentSelection.Export -> pending.copy(uri = uri)
            null -> null
        }
    }
    fun discardAttachmentSelection() { attachmentSelection?.let(::requestUnpin); attachmentSelection = null }
    private fun requestUnpin(selection: AttachmentSelection) {
        val draft = when (selection) {
            is AttachmentSelection.Import -> selection.draft
            is AttachmentSelection.Export -> selection.draft
        } ?: return
        pendingUnpins.add(selection.database to draft)
        bound(::releaseAbandonedPins)
    }
    private suspend fun releaseAbandonedPins(binding: Binding) {
        for (identity in pendingUnpins.toList().filter { it.first == binding.database }) {
            io { binding.session.unpinForm(identity.second) }
            if (!current(binding)) return
            pendingUnpins.remove(identity)
        }
    }
    fun acceptAttachmentSelection() {
        val pending = attachmentSelection ?: return
        val uri = pending.uri ?: return
        val binding = binding() ?: return
        val expectedDraft = when (pending) { is AttachmentSelection.Import -> pending.draft; is AttachmentSelection.Export -> pending.draft }
        if (binding.database != pending.database || (expectedDraft != null && activeForm?.draft != expectedDraft) || (pending is AttachmentSelection.Import && entry == null)) {
            error = true; return
        }
        when (pending) {
            is AttachmentSelection.Import -> mutateAttachment("attachment:import:${pending.operation}", pending) { native ->
                val name = SelectedDescriptors.displayName(app, uri) ?: app.getString(R.string.attachment_file)
                native.importAttachment(pending.draft, name, pending.replacement, pending.operation, SelectedDescriptors.openInput(app, uri))
            }
            is AttachmentSelection.Export -> bound { fresh ->
                if (attachmentSelection != pending || fresh.database != pending.database) return@bound
                io { fresh.session.exportAttachment(pending.entry, pending.revision, pending.blob, SelectedDescriptors.openOutput(app, uri)) }
                if (current(fresh) && attachmentSelection == pending) { requestUnpin(pending); attachmentSelection = null; error = false }
            }
        }
    }
    fun renameAttachment(attachment: String, name: String) {
        val draft = activeForm?.draft ?: return
        val operation = newOperation()
        mutateAttachment("attachment:rename:$attachment") { it.renameAttachment(draft, attachment, name, operation) }
    }
    fun removeAttachment(attachment: String) {
        val draft = activeForm?.draft ?: return
        val operation = newOperation()
        mutateAttachment("attachment:remove:$attachment") { it.removeAttachment(draft, attachment, operation) }
    }
    private fun mutateAttachment(address: String, expectedSelection: AttachmentSelection.Import? = null, action: (DocumentSession) -> EntryEditor) {
        var response: EntryEditor? = null
        var rows: List<AttachmentRow> = emptyList()
        patch(address, {
            val input = entry
            val view = response
            if (input != null && view != null && input.source.form.draft == view.form.draft) {
                entry = input.copy(source = view); attachments = rows
                if (expectedSelection != null && attachmentSelection == expectedSelection) { requestUnpin(expectedSelection); attachmentSelection = null }
            }
        }) { native ->
            response = action(native)
            rows = native.formAttachments()
            response!!.form
        }
    }
    fun editMetadata(name: String? = null, description: String? = null, clearDescription: Boolean = false) {
        val input = metadata ?: return
        if (navigating) return
        metadata = input.copy(name = name ?: input.name, description = description ?: input.description)
        val form = input.source.form
        patch(if (name != null) "metadata:name" else "metadata:description") {
            it.patchMetadata(form.draft, form.kind, name?.let(TextEdit::Set) ?: TextEdit.Keep,
                if (clearDescription) TextEdit.Clear else description?.let(TextEdit::Set) ?: TextEdit.Keep).form
        }
    }
    fun editAttribute(id: String?, name: String, value: String?, protected: Boolean, remove: Boolean = false) {
        var response: EntryEditor? = null
        patch("attribute:${id ?: name}", {
            val view = response
            val input = entry
            if (view != null && input != null && input.source.form.draft == view.form.draft) entry = input.copy(source = view)
        }) { native ->
            native.patchAttribute(id, name, value?.let(TextEdit::Set) ?: TextEdit.Keep, protected, remove).also { response = it }.form
        }
    }
    private fun patch(address: String, applied: (() -> Unit)? = null, action: (DocumentSession) -> LocalForm) {
        val draft = activeForm?.draft ?: return
        activity()
        val expectedInput = ++inputRevision
        saveLabel = SaveLabel.DIRTY; error = false
        bound { binding ->
            if (activeForm?.draft != draft) return@bound
            retryEdits[address] = { native ->
                val updated = io { action(native) }
                if (current(binding)) { activeForm = updated; appliedInputRevision = maxOf(appliedInputRevision, expectedInput); applied?.invoke() }
            }
            val updated = io { action(binding.session) }
            if (current(binding) && activeForm?.draft == draft) {
                activeForm = updated; appliedInputRevision = maxOf(appliedInputRevision, expectedInput); retryEdits.remove(address); applied?.invoke()
                if (inputRevision == expectedInput) saveLabel = SaveLabel.DIRTY
            }
        }
        saveJob?.cancel()
        saveJob = viewModelScope.launch { delay(500); saveNow() }
    }
    fun saveNow() { saveJob?.cancel(); bound { flush(it) } }
    private suspend fun flush(binding: Binding) {
        for ((address, retry) in retryEdits.toMap()) {
            retry(binding.session)
            if (!current(binding)) return
            retryEdits.remove(address)
        }
        val capturedInput = appliedInputRevision
        val captured = activeForm ?: return
        if (!captured.dirty) return
        val operation = receipts.getOrPut(captured.draft to captured.revision, ::newOperation)
        saveLabel = SaveLabel.SAVING
        try {
            val saved = io { binding.session.save(captured.draft, captured.revision, operation) }
            if (!current(binding) || activeForm?.draft != captured.draft || saved.database != binding.database || saved.generation != binding.generation) return
            // Refresh identity only. Keyboard input and focus belong to this existing UI form.
            val updated = if (entry != null) {
                val view = io { binding.session.editor() }
                if (!current(binding)) return
                entry = entry?.copy(source = view); view.form
            } else {
                val view = io { binding.session.metadata(captured.draft) }
                if (!current(binding)) return
                metadata = metadata?.copy(source = view); view.form
            }
            activeForm = updated
            if (captured.kind == FormKind.NEW_GROUP && saved.state == SaveState.SAVED) { selectedGroup = updated.target; ungrouped = false }
            saveLabel = if (inputRevision != capturedInput) SaveLabel.DIRTY else when (saved.state) {
                SaveState.SAVED -> SaveLabel.SAVED
                SaveState.LOCAL_DRAFT_SAVED -> SaveLabel.LOCAL
                SaveState.UNCHANGED -> SaveLabel.CLEAN
            }
            error = false; refresh(binding)
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (failure: Exception) {
            if (binding.session.access() != SessionAccess.OPEN) {
                if (session === binding.session && epoch == binding.epoch) clearSensitive()
                return
            }
            io { binding.session.persistForms() }
            if (current(binding)) { error = true; saveLabel = SaveLabel.LOCAL }
        }
    }
    fun showHistory(kind: FormKind, target: String?) = navigate { binding ->
        val rows = io { binding.session.history(kind, target) }
        if (current(binding)) { history = HistorySelection(kind, target, rows); historicalEntry = null; revealed = "" }
    }
    fun selectRevision(row: HistoryRow) {
        hideSecrets(); historicalEntry = null; historicalRevision = row.id; historicalAttachments = emptyList(); historicalAttributes = emptyList()
        bound { binding ->
            val selection = history ?: return@bound
            if (selection.kind == FormKind.ENTRY && selection.target != null) {
                val view = io { binding.session.revision(selection.target, row.id) }
                val rows = io { binding.session.entryAttachments(selection.target, row.id) }
                val attributes = io { binding.session.entryAttributes(selection.target, row.id) }
                if (current(binding) && historicalRevision == row.id) { historicalEntry = view; historicalAttachments = rows; historicalAttributes = attributes }
            }
        }
    }
    fun dismissHistory() { history = null; historicalEntry = null; historicalRevision = null; historicalAttachments = emptyList(); historicalAttributes = emptyList(); hideSecrets() }
    fun reveal(attribute: String? = null) {
        val request = secretEpoch
        bound { binding ->
            val id = history?.target ?: entry?.source?.entry ?: inspectedEntry?.id ?: return@bound
            val revision = historicalRevision
            val value = io { binding.session.reveal(id, revision, attribute) }
            if (current(binding) && revision == historicalRevision && secretEpoch == request) revealed = value
        }
    }
    fun hideSecrets() { secretEpoch++; revealed = ""; invitation = "" }
    fun purgeHistory(selection: HistorySelection, revisions: List<String>, operation: String) = bound { binding ->
        io { binding.session.purgeHistory(selection.kind, selection.target, revisions, operation) }
        if (current(binding)) {
            history = HistorySelection(selection.kind, selection.target, io { binding.session.history(selection.kind, selection.target) })
            historicalEntry = null; historicalRevision = null; historicalAttachments = emptyList(); historicalAttributes = emptyList(); hideSecrets()
        }
    }
    fun restoreRevision(entry: String, revision: String, operation: String) = navigate { binding ->
        val group = inspectedEntry?.group ?: this.entry?.source?.group
        val restored = io { binding.session.restoreRevision(entry, revision, group, operation) }
        val view = io { binding.session.entry(restored) }
        if (current(binding)) { clearEditor(); inspectedEntry = view; dismissHistory(); loadEntryTab(binding) }
    }
    fun trash(kind: FormKind, target: String, group: String?) = navigate { binding ->
        io { binding.session.trash(kind, target, newOperation()) }
        if (current(binding)) { undo = UndoTarget(kind, target, group); clearEditor() }
    }
    fun undoTrash() = bound { binding ->
        val removed = undo ?: return@bound
        io { binding.session.undoTrash(removed.kind, removed.target, removed.group, newOperation()) }
        if (current(binding)) { undo = null; refresh(binding) }
    }
    fun exchange() {
        startExchange { request ->
            val host = app.host.await()
            val view = io { host.exchangeView(overview?.database) }
            val requests = io { host.pendingJoins() }
            if (epoch == request) { exchange = view; pending = requests }
        }
    }
    private fun startExchange(action: suspend (Long) -> Unit) {
        val request = epoch
        ExchangeScheduler.startForeground(app) { started ->
            if (epoch == request) {
                if (!started) error = true
                else enqueue { if (epoch == request) action(request) }
            }
        }
    }
    fun dismissExchange() { exchange = null; hideSecrets() }
    fun invite() {
        hideSecrets()
        val request = secretEpoch
        val binding = binding() ?: return
        startExchange {
            if (!current(binding) || secretEpoch != request) return@startExchange
            val code = io { binding.session.createInvitation() }
            if (current(binding) && secretEpoch == request) {
                invitation = code; exchange = io { app.host.awaitBlocking().exchangeView(binding.database) }
                viewModelScope.launch { delay(300_000); if (secretEpoch == request) invitation = "" }
            }
        }
    }
    fun approve(request: String) = bound { binding ->
        io { binding.session.approveInvitation(request) }
        if (current(binding)) exchange = io { app.host.awaitBlocking().exchangeView(binding.database) }
    }
    fun reject(request: String) = bound { binding ->
        io { binding.session.closeInvitation(request, true) }
        if (current(binding)) exchange = io { app.host.awaitBlocking().exchangeView(binding.database) }
    }
    fun join(code: String) = startExchange { request ->
        val owner = manager.await()
        io { owner.join(code) }
        val copies = io { owner.catalog() }; val requests = io { owner.pendingJoins() }
        if (epoch == request) { catalog = copies; pending = requests }
    }
    fun resumeJoin(request: String) = startExchange { requestedEpoch ->
        val owner = manager.await()
        io { owner.resumeJoin(request) }
        val copies = io { owner.catalog() }; val requests = io { owner.pendingJoins() }
        if (epoch == requestedEpoch) { catalog = copies; pending = requests }
    }
    fun showPolicy() = bound { binding ->
        val view = io { binding.session.policy() }; if (current(binding)) policy = view
    }
    fun dismissPolicy() { policy = null }
    fun setPolicy(value: PolicyView, password: String?, operation: String) = bound { binding ->
        io { binding.session.setPolicy(value, password, operation) }
        if (current(binding)) { policy = null; refresh(binding) }
    }
    fun rotate(password: String, revoke: String?, operation: String) = bound { binding ->
        io { binding.session.rotatePassword(password, revoke, operation) }
        if (session === binding.session && epoch == binding.epoch) clearSensitive()
    }
    fun clearError() { error = false }
    fun reportFailure() { error = true }
    fun activity() { manager.getCompletedOrNull()?.activity() }
    fun lock() {
        session?.lock()
        val owner = manager.getCompletedOrNull()
        if (owner == null) clearSensitive() else owner.background()
    }
    private fun clearEditor() {
        entry = null; inspectedEntry = null; metadata = null; activeForm = null
        appearance = null; properties = null; entryHistory = emptyList()
        inspectedAttributes = emptyList(); attachments = emptyList()
        historicalAttachments = emptyList(); historicalAttributes = emptyList()
        retryEdits.clear(); hideSecrets(); inputRevision++; saveLabel = SaveLabel.CLEAN
    }
    private fun clearContent() { clearEditor(); overview = null; forms = emptyList(); history = null; historicalEntry = null; historicalRevision = null; policy = null; undo = null; query = ""; selectedGroup = null; ungrouped = false; hideSecrets() }
    private fun clearSensitive() {
        epoch++; secrecyEpoch++; saveJob?.cancel(); clearContent(); opening = false; navigating = false; error = false
        val clearedEpoch = epoch
        val old = session; session = null
        while (commands.tryReceive().isSuccess) { /* Drop pending entered JVM values immediately. */ }
        if (old != null) viewModelScope.launch {
            try { io { manager.awaitBlocking().close(old) } }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == clearedEpoch && session == null) error = true }
        }
    }
    override fun onCleared() {
        val owner = manager.getCompletedOrNull()
        lock(); commands.close()
        if (owner != null) cleanup.launch { try { owner.close() } catch (_: Exception) { /* Native supervisor owns completion status. */ } finally { cleanup.cancel() } }
        else cleanup.cancel()
        super.onCleared()
    }
}

fun newOperation(): String = UUID.randomUUID().toString().replace("-", "") + UUID.randomUUID().toString().replace("-", "")
private fun <T> Deferred<T>.awaitBlocking(): T = runBlocking { await() }
@OptIn(ExperimentalCoroutinesApi::class)
private fun <T> Deferred<T>.getCompletedOrNull(): T? = if (isCompleted && !isCancelled) getCompleted() else null
