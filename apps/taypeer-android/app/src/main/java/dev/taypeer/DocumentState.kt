package dev.taypeer

import android.app.Application
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableStateMapOf
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
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.UUID

enum class SaveLabel { CLEAN, DIRTY, SAVING, SAVED, FAILED }
enum class RotationOutcome { CONFIRMED, REJECTED, UNCERTAIN }
data class EntryInput(val source: EntryEditor, val title: String = source.title,
    val username: String = source.username.orEmpty(), val password: String = "",
    val url: String = source.url.orEmpty(), val notes: String = source.notes.orEmpty(),
    val expiry: String = source.expiryInput ?: source.expiresAt?.toString().orEmpty(), val expiryEnabled: Boolean = source.expiryInput != null || source.expiresAt != null,
    val tags: String = source.tags.joinToString("\n"))
data class MetadataInput(val source: MetadataEditor, val name: String = source.name,
    val description: String = source.description.orEmpty(), val parent: String? = source.parent,
    val parentRevision: Long = 0L)
data class HistorySelection(val kind: FormKind, val target: String?, val rows: List<HistoryRow>, val group: String? = null)
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
class DocumentState internal constructor(application: Application, private val iconExecutor: IconStageExecutor,
    internal val accesses: SecretAccessState = (application as TaypeerApplication).secretAccess) : AndroidViewModel(application) {
    constructor(application: Application) : this(application, NativeIconStageExecutor(application))
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
    /** Fence delayed context actions across lock and same-copy session replacement. */
    internal val contextSessionEpoch get() = epoch
    private var inputRevision = 0L
    private var appliedInputRevision = 0L
    private var secretEpoch = 0L
    private var secretReadSequence = 0L
    private val accessOwner = UUID.randomUUID().toString()
    private var displayIdentity: Any? = null
    internal var secretDisplay by mutableStateOf<SecretDisplayTicket?>(null); private set
    private val cleanup = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val receipts = mutableMapOf<Pair<String, ULong>, String>()
    private val saveAttempts = mutableMapOf<String, SaveAttempt>()
    private var pendingNavigation: PendingNavigation? = null
    private val retryEdits = linkedMapOf<String, suspend (DocumentSession) -> Unit>()
    private val pendingUnpins = linkedSetOf<Pair<String, String>>()
    var catalog by mutableStateOf<List<WorkingCopyView>>(emptyList()); private set
    var lockedCopy by mutableStateOf<WorkingCopyView?>(null); private set
    fun chooseAnotherDatabase() { biometry.clear(); groupFavicons.reset(); databaseCopy.cancel(); copySource = null; lockedCopy = null; searches.close() }
    internal val biometry = BiometryUiState(viewModelScope) { app.host.await() }
    internal fun loadBiometry(copy: WorkingCopyView? = currentCopy()) { copy?.let(biometry::load) }
    internal fun enableBiometry() {
        val copy = currentCopy() ?: return
        val active = session ?: return
        biometry.enable(copy, active)
    }
    internal fun disableBiometry() { currentCopy()?.let(biometry::disable) }
    internal fun retryBiometry() { currentCopy()?.let(biometry::retry) }
    private fun currentCopy() = catalog.singleOrNull { it.database == overview?.database }
    private var copySource: WorkingCopyView? = null
    internal val databaseCopy = CiphertextCopyState(viewModelScope, { app.host.await() }, currentSource = {
        copySource?.takeIf { selected -> catalog.any { it.database == selected.database && it.root == selected.root && it.path == selected.path } }
    }, output = { SelectedDescriptors.openOutput(app, it) })
    /** Explicitly selected catalog source, independent of the plaintext worker's generation. */
    internal fun beginDatabaseCopy(source: WorkingCopyView? = catalog.singleOrNull { it.database == (overview?.database ?: lockedCopy?.database) }) {
        val selected = source ?: return
        if (catalog.none { it.database == selected.database && it.root == selected.root && it.path == selected.path }) return
        hideSecrets()
        copySource = selected
        val filename = overview?.takeIf { it.database == selected.database }?.name?.let { "$it.taypeer" }
            ?: app.getString(R.string.copy_default_filename)
        databaseCopy.begin(selected, filename)
    }
    var overview by mutableStateOf<DocumentOverview?>(null); private set
    var entry by mutableStateOf<EntryInput?>(null); private set
    var inspectedEntry by mutableStateOf<EntryRow?>(null); private set
    var metadata by mutableStateOf<MetadataInput?>(null); private set
    var forms by mutableStateOf<List<LocalForm>>(emptyList()); private set
    var activeForm by mutableStateOf<LocalForm?>(null); private set
    var history by mutableStateOf<HistorySelection?>(null); private set
    var historicalEntry by mutableStateOf<EntryRow?>(null); private set
    var historicalRevision by mutableStateOf<String?>(null); private set
    var historicalProperties by mutableStateOf<EntryProperties?>(null); private set
    var historicalTab by mutableIntStateOf(0); private set
    var revealed by mutableStateOf(""); private set
    var revealedAttribute by mutableStateOf<String?>(null); private set
    var invitation by mutableStateOf(""); private set
    var exchange by mutableStateOf<ExchangeView?>(null); private set
    var pending by mutableStateOf<List<EnrollmentRow>>(emptyList()); private set
    val policy get() = policySettings.details?.policy
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
    internal var entrySort by mutableStateOf(EntrySort.TITLE_ASC); private set
    internal fun selectEntrySort(value: EntrySort) {
        if (entrySort == value) return
        entrySort = value
        searches.resort()
    }
    internal var unlockedCopies by mutableStateOf<List<WorkingCopyView>>(emptyList()); private set
    private val sourceGroups = mutableMapOf<SearchSourceKey, String?>()
    internal val searches = SearchState(viewModelScope,
        sources = { manager.getCompletedOrNull()?.unlockedSources().orEmpty() },
        selected = { session?.let { manager.getCompletedOrNull()?.unlockedSource(it) } },
        selectedGroup = { selectedGroup }, selectedName = { overview?.name }, order = { entrySort })
    val query get() = searches.query
    var saveLabel by mutableStateOf(SaveLabel.CLEAN); private set
    var error by mutableStateOf(false); private set
    var opening by mutableStateOf(false); private set
    var navigating by mutableStateOf(false); private set
    var navigationDecision by mutableStateOf(false); private set
    var invalidInput by mutableStateOf(false); private set
    val editingBlocked get() = navigating || navigationDecision || icons.blocked(iconTarget())
    var generator by mutableStateOf(false)
    var secrecyEpoch by mutableStateOf(0L); private set
    var applicationStatus by mutableStateOf<ApplicationState?>(null); private set
    private var observedApplicationBinding: Binding? = null
    private var observedApplicationRevision: ULong? = null
    private var observedAppliedRevision = 0uL
    private var incomingRefreshQueued = false
    private val sessionCommands = object : SessionCommands {
        override fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
            completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit, discarded: (T) -> Unit): Boolean {
            if (binding() == null || (navigate && editingBlocked)) return false
            val action: suspend (Binding) -> Unit = action@ { binding ->
                started()
                val result = try {
                    ownedIo(discarded) { command(binding.session) }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (failure: Exception) {
                    if (current(binding)) {
                        if (failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions) rejected()
                        else failed()
                    }
                    return@action
                }
                // A failed later projection must not turn a durable native reply into a failed command.
                if (current(binding)) { completed(result); refresh(binding) }
                else discarded(result)
            }
            if (navigate) this@DocumentState.navigate { binding -> clearEditor(); action(binding) }
            else bound(action)
            return true
        }
    }
    val lifecycle = DocumentLifecycleState(sessionCommands, database = { binding()?.database }, selectDestination = { selectedGroup = it })
    val received = ReceivedSourcesState(sessionCommands, database = { binding()?.database })
    val comparison = EntryComparisonState(sessionCommands, ::accessTicket)
    internal val policySettings = PolicyState(sessionCommands, source = {
        binding()?.let { selected -> manager.getCompletedOrNull()?.unlockedSource(selected.session)?.let { source ->
            PolicySource(selected.database, source.copy.path, selected.generation)
        } }
    }, canManage = { overview?.managing == true }, confirmed = { loadBiometry() })
    val restorations = RestoreRevisionState(sessionCommands, database = { binding()?.database }) { restored ->
        bound { binding ->
            val view = io { binding.session.entry(restored) }
            if (current(binding)) { clearEditor(); inspectedEntry = view; dismissHistory(); loadEntryTab(binding) }
        }
    }
    internal val metadataHistory = MetadataHistoryState(sessionCommands, database = { binding()?.database }) { kind, target ->
        bound { binding ->
            val objectId = if (kind == FormKind.GROUP) target else null
            val rows = io { binding.session.history(kind, objectId) }
            if (current(binding)) {
                clearEditor(); comparison.close()
                history = HistorySelection(kind, objectId, rows)
                historicalEntry = null; historicalRevision = null
                refresh(binding)
            }
        }
    }
    internal val recovery = RecoveryState(viewModelScope, { manager.await() }, ::detachRecoverySource) { copy, password ->
        lockedCopy = copy
        openResolved { it.open(copy.path, password) }
    }
    internal fun beginRecovery(resume: (PendingRecoveryView) -> Unit) = navigate { selected ->
        clearEditor()
        val source = catalog.singleOrNull { it.database == selected.database } ?: return@navigate
        val retained = io { manager.awaitBlocking().pendingRecoveries() }
            .singleOrNull { it.source.path == source.path && it.source.database == source.database }
        if (!current(selected)) return@navigate
        hideSecrets()
        if (retained != null) resume(retained) else recovery.begin(source, selected.session)
    }
    internal fun authenticateRecovery(source: WorkingCopyView, password: String, operation: String) {
        leaveEditor { recovery.authenticate(source, password, operation) }
    }
    private fun detachRecoverySource() {
        databaseCopy.cancel(); copySource = null
        val previous = session
        val previousDatabase = overview?.database
        session = null; epoch++; clearContent(); opening = false; navigating = false
        // A locked catalog row may target another database. Its recovery owner closes
        // that source; the detached foreground session still needs its own close.
        if (previous != null && previousDatabase != recovery.target?.source?.database) {
            val expected = epoch
            viewModelScope.launch {
                try { io { manager.awaitBlocking().close(previous) } }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { if (epoch == expected && session == null) error = true }
            }
        }
    }

    internal val management = ManagementState(viewModelScope, { app.host.await() }, session = {
        binding()?.let { ManagementUiSession(it.session, it.database, it.generation, it.epoch) }
    }, current = { selected ->
        binding()?.let { it.session === selected.session && it.database == selected.database &&
            it.generation == selected.generation && it.epoch == selected.epoch && current(it) } == true
    }, changed = { expected ->
        bound { selected ->
            if (selected.session === expected.session && selected.database == expected.database &&
                selected.generation == expected.generation && selected.epoch == expected.epoch) refresh(selected)
        }
    })
    internal fun beginManagementTransfer(recipient: String? = null) = navigate { selected ->
        clearEditor(); hideSecrets(); management.selectDatabase(selected.database); management.openTransfer(recipient)
    }
    internal fun reviewManagement(row: ManagementHandoffView) = leaveEditor {
        hideSecrets(); management.openDecision(row)
    }

    internal val groupFavicons = GroupFaviconsState(viewModelScope, binding = {
        binding()?.let { GroupFaviconBinding(it.session, it.database, it.generation, it.epoch) }
    }, current = { expected ->
        binding()?.let { it.session === expected.session && it.database == expected.database &&
            it.generation == expected.generation && it.epoch == expected.epoch && current(it) } == true
    }, changed = { expected ->
        bound { selected ->
            if (selected.session === expected.session && selected.database == expected.database &&
                selected.generation == expected.generation && selected.epoch == expected.epoch) {
                clearIconPreviews(); refresh(selected)
            }
        }
    })
    internal fun beginGroupFavicons(group: String) = navigate { selected ->
        val target = overview?.groups?.singleOrNull { it.id == group } ?: return@navigate
        if (!current(selected) || overview?.writable != true) return@navigate
        clearEditor(); hideSecrets(); groupFavicons.open(target)
    }

    internal val icons = IconAcquisitionState()
    internal var storedFavicon by mutableStateOf<StoredFaviconView?>(null); private set
    internal var iconPreviewEpoch by mutableStateOf(0L); private set
    private data class IconPreviewKey(val database: String, val generation: ULong, val scope: IconScope, val identity: String)
    private val iconThumbnails = mutableStateMapOf<IconPreviewKey, RenderedIcon>()
    private val iconScopes = mutableStateMapOf<IconScope, IconPreviewKey>()
    private val iconPreviewOrder = linkedSetOf<IconPreviewKey>()
    private val iconPreviewRequests = mutableMapOf<IconScope, Long>()
    // Cancellation can withdraw offscreen waiters here, before blocking JNI.
    // Serialize the complete preview so its projection and export stay together.
    private val iconPreviewMutex = Mutex()
    // Bounded, content-free diagnostic for real worker preview failures; never logged by product UI.
    private val iconPreviewPhases = linkedMapOf<IconScope, String>()
    internal fun iconPreviewPhase(scope: IconScope): String = iconPreviewPhases[scope] ?: "not-requested"
    internal fun readPreviewDiagnostic(read: (DocumentSession) -> String): String = binding()?.let { read(it.session) } ?: "no-open-worker"
    private fun previewPhase(scope: IconScope, phase: String) {
        iconPreviewPhases.remove(scope); iconPreviewPhases[scope] = phase
        while (iconPreviewPhases.size > 64) iconPreviewPhases.remove(iconPreviewPhases.keys.first())
    }
    internal fun iconTarget(): IconTarget? = overview?.database?.let { database -> activeForm?.draft?.let { IconTarget(database, it) } }
    internal fun renderedIcon(scope: IconScope): RenderedIcon? = iconScopes[scope]?.let(iconThumbnails::get)
    internal suspend fun loadIconPreview(scope: IconScope) {
        val selected = binding() ?: return
        val expectedEpoch = iconPreviewEpoch
        val request = (iconPreviewRequests[scope] ?: 0L) + 1L
        iconPreviewRequests[scope] = request
        previewPhase(scope, "queued")
        var value: RenderedIcon? = null
        var failure: String? = null
        try {
            iconPreviewMutex.withLock {
                currentCoroutineContext().ensureActive()
                if (!current(selected) || iconPreviewEpoch != expectedEpoch || iconPreviewRequests[scope] != request) return
                previewPhase(scope, "pending")
                io { value = selected.session.iconPreview(scope) }
            }
        }
        catch (cancelled: CancellationException) { value?.png?.fill(0); throw cancelled }
        catch (error: Exception) { value?.png?.fill(0); value = null; failure = error.javaClass.simpleName }
        if (!current(selected) || iconPreviewEpoch != expectedEpoch || iconPreviewRequests[scope] != request) { value?.png?.fill(0); return }
        val thumbnail = value
        previewPhase(scope, if (failure != null) "failed:$failure" else if (thumbnail == null) "no-image" else "ready")
        if (thumbnail == null) iconScopes.remove(scope)
        else {
            val key = IconPreviewKey(selected.database, selected.generation, scope, thumbnail.identity)
            iconScopes[scope]?.takeIf { it != key }?.let { old -> iconThumbnails.remove(old)?.png?.fill(0); iconPreviewOrder.remove(old) }
            iconThumbnails.put(key, thumbnail)?.takeIf { it !== thumbnail }?.png?.fill(0); iconScopes[scope] = key
            iconPreviewOrder.remove(key); iconPreviewOrder.add(key)
            while (iconPreviewOrder.size > 64) {
                val removed = iconPreviewOrder.first(); iconPreviewOrder.remove(removed)
                iconThumbnails.remove(removed)?.png?.fill(0)
                iconScopes.keys.toList().filter { iconScopes[it] == removed }.forEach(iconScopes::remove)
            }
        }
    }
    private fun clearIconPreviews() {
        iconPreviewEpoch++; iconScopes.clear(); iconPreviewRequests.clear(); iconPreviewOrder.clear(); iconPreviewPhases.clear()
        iconThumbnails.values.forEach { it.png.fill(0) }; iconThumbnails.clear(); storedFavicon = null
    }
    internal fun loadStoredFavicon() {
        val target = iconTarget() ?: return
        storedFavicon = null
        bound { selected ->
            if (iconTarget() != target) return@bound
            val value = io { selected.session.storedFavicon(target.draft) }
            if (current(selected) && iconTarget() == target) storedFavicon = value
        }
    }
    internal fun prepareIconFile(launch: (String) -> Unit) {
        val target = iconTarget() ?: return
        if (editingBlocked || overview?.writable != true) return
        navigating = true
        bound { selected ->
            replayEdits(selected)
            icons.discardFile()?.let { old -> pendingUnpins.add(old.target.database to old.target.draft) }
            releaseAbandonedPins(selected)
            val pinned = io { iconExecutor.pin(selected.session) }
            if (current(selected) && iconTarget() == target && pinned.draft == target.draft) {
                icons.selectFile(IconFileSelection(target, newOperation()))
                navigating = false; launch(icons.file!!.operation)
            }
        }
    }
    internal fun selectedIconFile(operation: String?, uri: Uri?) {
        val selected = icons.file ?: return
        if (operation == null || selected.operation != operation || icons.attempt(selected.target) != null) return
        if (uri == null) discardIconFile() else icons.selectFile(selected.copy(uri = uri))
    }
    internal fun discardIconFile() {
        val selected = icons.file ?: return
        if (icons.attempt(selected.target) != null) return
        icons.discardFile()?.let { pendingUnpins.add(it.target.database to it.target.draft) }
        bound(::releaseAbandonedPins)
    }
    internal fun acceptIconFile() {
        val selected = icons.file ?: return
        val uri = selected.uri ?: return
        if (selected.target != iconTarget() || overview?.writable != true) return
        beginIconRequest(IconRequest(selected.target, IconSource.File(uri), selected.operation))
    }
    internal fun downloadIcon(url: String, favicon: Boolean = false) {
        val target = iconTarget() ?: return
        if (editingBlocked || overview?.writable != true) return
        val source = if (favicon) IconDownload.Favicon(url) else IconDownload.ImageUrl(url)
        beginIconRequest(IconRequest(target, IconSource.Download(source), newOperation()))
    }
    internal fun selectStoredFavicon(identity: String) {
        val target = iconTarget() ?: return
        if (editingBlocked || overview?.writable != true) return
        beginIconRequest(IconRequest(target, IconSource.Stored(identity), newOperation()))
    }
    internal fun retryIcon() {
        val attempt = icons.attempt(iconTarget()) ?: return
        if (!attempt.busy && overview?.writable == true) beginIconRequest(attempt.request)
    }
    private fun beginIconRequest(request: IconRequest) {
        val selected = binding() ?: return
        if (request.target != iconTarget() || navigating || navigationDecision) return
        val token = icons.begin(request) ?: return
        activity()
        // Replay/pin uses the normal queue. Bounded HTTP and file acquisition do not hold it.
        enqueue {
            try {
                if (!current(selected) || request.target != iconTarget()) return@enqueue
                if (icons.attempt(request.target)?.prepared != true) {
                    replayEdits(selected)
                    val pinned = io { iconExecutor.pin(selected.session) }
                    if (!current(selected) || pinned.draft != request.target.draft || request.target != iconTarget()) return@enqueue
                    icons.prepared(request, token)
                }
                viewModelScope.launch {
                    try {
                        io { iconExecutor.execute(selected.session, request) }
                        // The receipt is an acknowledgement, not a projection of today's form.
                        if (!icons.confirmed(request, token)) return@launch
                        pendingUnpins.add(request.target.database to request.target.draft)
                        if (current(selected) && request.target == iconTarget()) bound { fresh ->
                            if (fresh != selected || iconTarget() != request.target) return@bound
                            val editor = if (entry != null) io { fresh.session.editor() } else null
                            val descriptive = if (metadata != null) io { fresh.session.metadata(request.target.draft) } else null
                            val appearanceView = if (editor != null) io { fresh.session.formAppearance() } else null
                            if (current(fresh) && iconTarget() == request.target) {
                                editor?.let { value -> entry = entry?.copy(source = value); activeForm = value.form; appearance = appearanceView }
                                descriptive?.let { value -> metadata = metadata?.copy(source = value); activeForm = value.form }
                                clearIconPreviews(); saveLabel = if (activeForm?.dirty == true) SaveLabel.DIRTY else SaveLabel.CLEAN
                                error = false; releaseAbandonedPins(fresh); refresh(fresh)
                            }
                        }
                    } catch (cancelled: CancellationException) { throw cancelled }
                    catch (failure: Exception) {
                        val settled = icons.failed(request, token, failure)
                        if (settled && (failure is AndroidException.Icon || failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions)) {
                            pendingUnpins.add(request.target.database to request.target.draft); bound(::releaseAbandonedPins)
                        }
                    }
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (failure: Exception) { icons.failed(request, token, failure) }
        }
    }

    private data class Binding(val session: DocumentSession, val epoch: Long, val database: String, val generation: ULong)
    private data class SaveAttempt(val form: LocalForm, val operation: String, val input: Long)
    private data class SaveCompletion(val form: LocalForm, val input: Long)
    private data class PendingNavigation(val binding: Binding, val draft: String, val action: suspend (Binding) -> Unit)
    init {
        viewModelScope.launch {
            accesses.confirmations.collect { confirmed ->
                val selected = binding() ?: return@collect
                if (!current(selected)) return@collect
                val stamp = accessBinding(selected)
                val target = confirmed.target
                if (target.database == selected.database && confirmed.binding == stamp) {
                    properties?.takeIf { it.entry == target.entry }?.let { properties = it.copy(accessedAt = confirmed.time) }
                    historicalProperties?.takeIf { it.entry == target.entry }?.let { historicalProperties = it.copy(accessedAt = confirmed.time) }
                }
            }
        }
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
                val owner = manager.getCompletedOrNull()
                val sources = owner?.unlockedSources().orEmpty()
                val selected = session
                if (selected != null && selected.access() != SessionAccess.OPEN) clearSelectedSensitive()
                val copies = sources.map { it.copy }.sortedBy { it.database }
                if (unlockedCopies != copies) {
                    unlockedCopies = copies
                    sourceGroups.keys.retainAll(sources.map { SearchSourceKey(it.copy.database, it.copy.path, it.generation) }.toSet())
                    if (owner != null) viewModelScope.launch { io { owner.closeRevoked() } }
                }
                searches.reconcile()
                if (sources.isEmpty() && (searches.showing || searches.returning != null || searches.query.isNotEmpty())) searches.close()
            }
        }
        viewModelScope.launch {
            while (isActive) {
                delay(250)
                val selected = binding() ?: continue
                val status = try {
                    io { selected.session.applicationState() }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    continue
                } ?: continue
                if (!current(selected)) continue
                if (observedApplicationBinding != selected) {
                    observedApplicationBinding = selected; observedApplicationRevision = null
                    observedAppliedRevision = 0uL; incomingRefreshQueued = false
                }
                applicationStatus = status
                if (status.revision != observedApplicationRevision && !incomingRefreshQueued) {
                    incomingRefreshQueued = true
                    bound { binding ->
                        if (binding != selected) return@bound
                        try {
                            if (status.appliedRevision != observedAppliedRevision) {
                                refreshIncomingProjection(binding)
                            } else {
                                // Identical/failed application reports carry no new document mutation.
                                refresh(binding); received.refreshIfOpen()
                            }
                            if (current(binding)) {
                                observedApplicationRevision = status.revision
                                observedAppliedRevision = status.appliedRevision
                            }
                        } finally { if (current(binding)) incomingRefreshQueued = false }
                    }
                }
            }
        }
        viewModelScope.launch {
            while (isActive) {
                delay(1_000)
                management.selectDatabase(overview?.database ?: lockedCopy?.database)
                // Protected queries reconfirm their journal publication. Do not poll
                // indefinitely after lock; selection, lock and explicit actions refresh it.
                if (binding()?.let(::current) == true) management.load()
                val selected = exchange ?: continue
                val requestedEpoch = epoch
                try {
                    val value = io { app.host.awaitBlocking().exchangeView(selected.database) }
                    if (epoch == requestedEpoch && exchange?.database == selected.database && exchange != null) exchange = value
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { if (epoch == requestedEpoch && exchange != null) error = true }
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
    private data class OwnedReply<T>(val value: T)
    /** Own the reply inside IO so prompt cancellation before Main delivery still releases resources. */
    private suspend fun <T> ownedIo(discarded: (T) -> Unit, action: () -> T): T {
        var reply: OwnedReply<T>? = null
        try { return io { action().also { reply = OwnedReply(it) } } }
        catch (cancelled: CancellationException) { reply?.let { discarded(it.value) }; throw cancelled }
    }
    private fun enqueue(action: suspend () -> Unit) { commands.trySend(action) }
    private fun bound(action: suspend (Binding) -> Unit) {
        val binding = binding() ?: return
        enqueue {
            if (current(binding)) {
                try { action(binding) }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (session === binding.session && epoch == binding.epoch && binding.session.access() != SessionAccess.OPEN) clearSensitive()
                    else if (current(binding)) {
                        invalidInput = failure is AndroidException.InvalidInput
                        error = !invalidInput; saveLabel = SaveLabel.FAILED; navigating = false
                    }
                }
            }
        }
    }
    fun reloadCatalog() = enqueue {
        val owner = manager.await()
        val copies = io { owner.catalog() }
        val requests = io { app.host.awaitBlocking().pendingJoins() }
        catalog = copies; pending = requests
        recovery.reload()
    }
    fun create(name: String, description: String?, password: String, operation: String) = open {
        it.create(name, description, operation, password)
    }
    fun unlock(copy: WorkingCopyView, password: String) = open { it.open(copy.path, password) }
    internal fun unlockBiometric(copy: WorkingCopyView) = open { it.openBiometric(copy) }
    /** An already OPEN source is selected without another password or writer. */
    internal fun chooseDatabase(copy: WorkingCopyView, authenticate: (WorkingCopyView) -> Unit) {
        val action = {
            val source = manager.getCompletedOrNull()?.unlockedSource(copy)
            if (source != null) openResolved { source.session }
            else { searches.close(); authenticate(copy) }
        }
        if (binding() == null) action() else navigate { action() }
    }
    fun import(uri: Uri, password: String) = open { it.import(uri, password) }
    private fun open(action: (DocumentManager) -> DocumentSession) {
        if (binding() != null) {
            navigate { openResolved(action = action) }
        } else openResolved(action = action)
    }
    private fun openResolved(preserveSearch: Boolean = false, resumeEditor: Boolean = true,
        afterOpen: suspend (Binding) -> Unit = {}, action: (DocumentManager) -> DocumentSession) {
        databaseCopy.cancel(); copySource = null
        if (!preserveSearch) searches.close()
        val requestedEpoch = epoch
        val previousBinding = binding()
        opening = true; navigating = true; error = false; invalidInput = false
        enqueue {
            if (epoch != requestedEpoch) return@enqueue
            val owner = manager.await()
            if (previousBinding != null) {
                if (!current(previousBinding)) return@enqueue
            }
            val previous = session
            previous?.let { owner.unlockedSource(it) }?.let { source ->
                sourceGroups[SearchSourceKey(source.copy.database, source.copy.path, source.generation)] = selectedGroup
            }
            // A UI switch detaches its projection; other OPEN workers and their
            // encrypted local forms remain owned by DocumentManager.
            session = null
            val openingEpoch = ++epoch
            clearContent()
            if (previous != null && previous.access() != SessionAccess.OPEN) io { owner.close(previous) }
            val opened = io { action(owner) }
            if (epoch != openingEpoch) { opened.lock(); io { owner.close(opened) }; return@enqueue }
            val view = io { opened.overview("") }
            if (epoch != openingEpoch || opened.access() != SessionAccess.OPEN) {
                opened.lock(); io { owner.close(opened) }; return@enqueue
            }
            clearContent()
            session = opened; overview = view; lockedCopy = null; opening = false; navigating = false; databaseImport = null
            owner.unlockedSource(opened)?.let { source ->
                val group = sourceGroups[SearchSourceKey(source.copy.database, source.copy.path, source.generation)]
                selectedGroup = group?.takeIf { id -> view.groups.any { it.id == id } }
            }
            management.selectDatabase(view.database)
            val binding = binding() ?: return@enqueue
            releaseAbandonedPins(binding)
            val active = if (resumeEditor) io { opened.activeForm() } else null
            if (active != null) { io { opened.resumeForm(active.draft) }; loadForm(binding, active) }
            if (current(binding)) afterOpen(binding)
            refresh(binding)
            catalog = io { owner.catalog() }
        }
    }
    private suspend fun refresh(binding: Binding) {
        val view = io { binding.session.overview("") }
        val local = io { binding.session.forms() }
        if (current(binding) && view.database == binding.database && view.generation == binding.generation) {
            overview = view
            if (selectedGroup != null && view.groups.none { it.id == selectedGroup }) selectedGroup = null
            forms = local
            if (searches.showing) searches.refresh()
        }
    }
    private suspend fun refreshIncomingProjection(binding: Binding) {
        clearIconPreviews()
        comparison.refreshIfOpen()
        metadataHistory.refreshIfOpen()
        policySettings.refreshIfOpen()
        refresh(binding)
        received.refreshIfOpen()
        // Draft input, identity and focus stay untouched even when its saved source changes.
        if (!current(binding) || entry != null || metadata != null) return
        val expected = inspectedEntry?.id ?: return
        val view = try { io { binding.session.entry(expected) } }
            catch (_: AndroidException.InvalidInput) { null }
        val information = view?.let { io { binding.session.entryProperties(expected) } }
        if (!current(binding) || inspectedEntry?.id != expected || entry != null || metadata != null) return
        hideSecrets()
        inspectedEntry = view; properties = information; appearance = information?.appearance
        loadEntryTab(binding)
    }
    private suspend fun loadForm(binding: Binding, form: LocalForm) {
        val editor = if (form.kind == FormKind.ENTRY || form.kind == FormKind.NEW_ENTRY) io { binding.session.editor() } else null
        val descriptive = if (editor == null) io { binding.session.metadata(form.draft) } else null
        if (!current(binding)) return
        entry = editor?.let(::EntryInput)
        metadata = descriptive?.let(::MetadataInput)
        inspectedEntry = null
        activeForm = editor?.form ?: descriptive?.form
        if (editor != null) selectedGroup = editor.group
        inputRevision++; appliedInputRevision = inputRevision; revealed = ""; history = null; historicalEntry = null; historicalRevision = null
        saveLabel = if (activeForm?.dirty == true) SaveLabel.DIRTY else SaveLabel.CLEAN
        saveAttempts[form.draft]?.let { attempt ->
            if (attempt.form.revision == activeForm?.revision) saveAttempts[form.draft] = attempt.copy(input = inputRevision)
        }
        loadEntryTab(binding)
    }
    private fun navigate(action: suspend (Binding) -> Unit) {
        if (editingBlocked || binding() == null) return
        navigating = true
        bound { binding ->
            replayEdits(binding)
            if (!current(binding)) return@bound
            val form = activeForm
            if (form != null && hasUnsavedInput()) {
                pendingNavigation = PendingNavigation(binding, form.draft, action)
                navigationDecision = true; navigating = false
            } else {
                form?.let { discard(binding, it.draft) }
                if (current(binding)) { action(binding); navigating = false; if (current(binding)) refresh(binding) }
            }
        }
    }
    private fun hasUnsavedInput() = activeForm?.dirty == true || inputRevision != appliedInputRevision || retryEdits.isNotEmpty()
    fun stay() { pendingNavigation = null; navigationDecision = false; navigating = false; lifecycle.navigationStayed(); received.navigationStayed(); restorations.navigationStayed(); metadataHistory.navigationStayed() }
    fun saveNavigation() {
        val pending = pendingNavigation ?: return
        val clickedInput = inputRevision
        navigating = true
        bound { binding ->
            if (binding != pending.binding || activeForm?.draft != pending.draft) return@bound
            val saved = saveSnapshot(binding, clickedInput)
            if (saved == null || inputRevision != saved.input) { navigating = false; return@bound }
            discard(binding, saved.form.draft)
            if (current(binding)) {
                pendingNavigation = null; navigationDecision = false
                pending.action(binding); navigating = false; if (current(binding)) refresh(binding)
            }
        }
    }
    fun discardNavigation() {
        val pending = pendingNavigation ?: return
        navigating = true
        bound { binding ->
            if (binding != pending.binding || activeForm?.draft != pending.draft) return@bound
            discard(binding, pending.draft)
            if (current(binding)) {
                pendingNavigation = null; navigationDecision = false
                pending.action(binding); navigating = false; if (current(binding)) refresh(binding)
            }
        }
    }
    fun chooseGroup(group: String?) = navigate {
        searches.close(); selectedGroup = group; clearEditor()
    }
    fun leaveEditor(action: () -> Unit) {
        if (binding() == null) action() else navigate { clearEditor(); action() }
    }
    fun beginEntry(id: String? = null) = navigate { binding ->
        val editor = io {
            if (id != null) binding.session.beginEditEntry(id)
            else binding.session.beginCreateEntry(selectedGroup ?: throw AndroidException.InvalidInput())
        }
        if (current(binding)) { if (id == null) entryTab = 0; loadForm(binding, editor.form) }
    }
    fun selectEntry(id: String) = navigate { binding ->
        val view = io { binding.session.entry(id) }
        val information = io { binding.session.entryProperties(id) }
        if (current(binding)) { clearEditor(); inspectedEntry = view; properties = information; appearance = information.appearance; loadEntryTab(binding) }
    }
    fun beginMetadata(kind: FormKind, group: String? = null, parent: String? = selectedGroup) {
        navigate { binding ->
            val editor = io { binding.session.beginMetadata(kind, group, parent) }
            if (current(binding)) loadForm(binding, editor.form)
        }
    }
    fun resumeForm(form: LocalForm) = navigate { binding ->
        io { binding.session.resumeForm(form.draft) }; loadForm(binding, form)
    }
    fun deleteForm(form: LocalForm) {
        if (editingBlocked || binding() == null) return
        navigating = true; error = false
        bound { binding ->
            discard(binding, form.draft)
            if (current(binding)) { if (activeForm?.draft == form.draft) clearEditor(); refresh(binding); navigating = false }
        }
    }
    fun search(value: String) {
        activity()
        searches.update(value)
        if (value.isEmpty()) bound { refresh(it) }
    }
    internal fun selectSearchScope(scope: SearchScope) { activity(); searches.selectScope(scope) }
    internal fun selectSearchResult(row: SearchResultRow) = navigate { selected ->
        val source = searches.source(row) ?: run { searches.reconcile(); return@navigate }
        if (!searches.beginResult(row)) return@navigate
        if (source.session === selected.session) inspectSearchResult(selected, row)
        else openResolved(preserveSearch = true, resumeEditor = false,
            afterOpen = { inspectSearchResult(it, row) }) { source.session }
    }
    private suspend fun inspectSearchResult(selected: Binding, row: SearchResultRow) {
        val source = searches.source(row) ?: return
        if (source.session !== selected.session || !current(selected)) return
        val view = io { source.session.entry(row.entry.id) }
        val information = io { source.session.entryProperties(row.entry.id) }
        if (!current(selected) || !source.permitted()) return
        clearEditor(); selectedGroup = view.group; inspectedEntry = view
        properties = information; appearance = information.appearance; loadEntryTab(selected)
    }
    internal fun returnToSearch() = navigate {
        val context = searches.returning ?: return@navigate
        val source = searches.source(context)
        if (source != null && source.session !== session) openResolved(preserveSearch = true, resumeEditor = false,
            afterOpen = {
                clearEditor(); selectedGroup = context.group?.takeIf { id -> overview?.groups?.any { it.id == id } == true }
                searches.returnToResults()
            }) { source.session }
        else {
            clearEditor()
            if (source != null) selectedGroup = context.group?.takeIf { id -> overview?.groups?.any { it.id == id } == true }
            searches.returnToResults()
        }
    }
    fun edit(field: EntryTextField, value: String, clear: Boolean = false) {
        val input = entry ?: return
        if (editingBlocked) return
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
        if (editingBlocked) return
        entry = input.copy(expiry = value)
        patch("expiry") { it.patchExpiry(value).form }
    }
    fun editTags(value: String) {
        val input = entry ?: return
        if (editingBlocked) return
        entry = input.copy(tags = value)
        patch("tags") { it.patchTags(if (value.isEmpty()) emptyList() else value.split('\n')).form }
    }
    fun setIcon(key: String?) {
        if (editingBlocked || entry == null) return
        val operation = newOperation()
        clearIconPreviews()
        appearance = (appearance ?: AppearanceView(null, null, null, null)).copy(lucide = key, image = null)
        patch("icon") { it.setIcon(key, operation).form }
    }
    fun setColors(foreground: ColorEdit, background: ColorEdit) {
        if (editingBlocked || entry == null) return
        val operation = newOperation()
        val old = appearance ?: AppearanceView(null, null, null, null)
        appearance = old.copy(
            foreground = when (foreground) { ColorEdit.Clear -> null; ColorEdit.Keep -> old.foreground; is ColorEdit.Set -> foreground.rgba },
            background = when (background) { ColorEdit.Clear -> null; ColorEdit.Keep -> old.background; is ColorEdit.Set -> background.rgba })
        val address = if (foreground == ColorEdit.Keep) "colors:background" else if (background == ColorEdit.Keep) "colors:foreground" else "colors:both"
        patch(address) { it.setColors(foreground, background, operation).form }
    }
    fun setGroupIcon(key: String?) {
        if (editingBlocked) return
        val draft = activeForm?.draft ?: return
        var response: MetadataEditor? = null
        clearIconPreviews()
        patch("metadata:icon", {
            val input = metadata
            val view = response
            if (input != null && view != null && input.source.form.draft == view.form.draft) metadata = input.copy(source = view)
        }) { native ->
            val updated = native.patchGroupIcon(draft, key?.let(TextEdit::Set) ?: TextEdit.Clear)
            response = updated
            updated.form
        }
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
        if (editingBlocked) return
        entry = input.copy(expiryEnabled = enabled)
        patch("expiry") { it.patchExpiry(if (enabled) input.expiry else null).form }
    }
    fun prepareAttachment(replacement: String?, launch: (String) -> Unit) {
        val form = activeForm ?: return
        if (entry == null || editingBlocked) return
        navigating = true
        bound { binding ->
            replayEdits(binding)
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
        if (editingBlocked) return
        navigating = true
        bound { binding ->
            replayEdits(binding)
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
        if (editingBlocked) return
        metadata = input.copy(name = name ?: input.name, description = description ?: input.description)
        val form = input.source.form
        patch(if (name != null) "metadata:name" else "metadata:description") {
            it.patchMetadata(form.draft, form.kind, name?.let(TextEdit::Set) ?: TextEdit.Keep,
                if (clearDescription) TextEdit.Clear else description?.let(TextEdit::Set) ?: TextEdit.Keep).form
        }
    }
    fun editGroupParent(parent: String?) {
        val input = metadata ?: return
        if (editingBlocked || (input.source.form.kind != FormKind.GROUP && input.source.form.kind != FormKind.NEW_GROUP)) return
        val draft = input.source.form.draft
        val revision = input.parentRevision + 1L
        metadata = input.copy(parent = parent, parentRevision = revision)
        var response: MetadataEditor? = null
        patch("metadata:parent", {
            val view = response
            val entered = metadata
            if (view != null && entered != null && entered.source.form.draft == draft && view.form.draft == draft) {
                // A later selection belongs to the same form, but not to this ACK.
                metadata = entered.copy(source = view,
                    parent = if (entered.parentRevision == revision) view.parent else entered.parent)
            }
        }) { native ->
            val updated = native.patchGroupParent(draft, parent?.let(GroupParentEdit::Set) ?: GroupParentEdit.Clear)
            response = native.metadata(draft)
            updated
        }
    }
    fun editAttribute(id: String?, name: String, value: String?, protected: Boolean, remove: Boolean = false) {
        if (editingBlocked) return
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
        if (editingBlocked) return
        val draft = activeForm?.draft ?: return
        activity()
        val expectedInput = ++inputRevision
        saveLabel = SaveLabel.DIRTY; error = false; invalidInput = false
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
    }
    private suspend fun replayEdits(binding: Binding) {
        for ((address, retry) in retryEdits.toMap()) {
            retry(binding.session)
            if (!current(binding)) return
            retryEdits.remove(address)
        }
    }
    fun saveNow() {
        if (editingBlocked || saveLabel == SaveLabel.SAVING) return
        val clickedInput = inputRevision
        bound { binding ->
            val saved = saveSnapshot(binding, clickedInput) ?: return@bound
            if (inputRevision == saved.input) closeForm(binding, saved.form.draft)
        }
    }
    fun cancelForm() {
        if (editingBlocked) return
        val draft = activeForm?.draft ?: return
        navigating = true
        bound { binding -> closeForm(binding, draft) }
    }
    private suspend fun discard(binding: Binding, draft: String) {
        // The service reconciles any uncertain attempt before cancelling the form;
        // repeating Save here could commit a known unsaved snapshot.
        io { binding.session.deleteForm(draft) }
        saveAttempts.remove(draft)
        receipts.keys.removeAll { it.first == draft }
        attachmentSelection?.let { selected ->
            val selectedDraft = when (selected) {
                is AttachmentSelection.Import -> selected.draft
                is AttachmentSelection.Export -> selected.draft
            }
            if (selected.database == binding.database && selectedDraft == draft) attachmentSelection = null
        }
        if (icons.file?.target == IconTarget(binding.database, draft)) icons.discardFile()
        pendingUnpins.remove(binding.database to draft)
    }
    private suspend fun closeForm(binding: Binding, draft: String) {
        if (!current(binding) || activeForm?.draft != draft) return
        navigating = true
        val id = entry?.source?.entry
        discard(binding, draft)
        val view = id?.let { io { binding.session.entry(it) } }
        val information = id?.let { io { binding.session.entryProperties(it) } }
        if (current(binding)) {
            clearEditor(); inspectedEntry = view; properties = information; appearance = information?.appearance
            navigating = false; refresh(binding); loadEntryTab(binding)
        }
    }
    private suspend fun saveSnapshot(binding: Binding, clickedInput: Long): SaveCompletion? {
        replayEdits(binding)
        if (!current(binding)) return null
        val captured = activeForm ?: return null
        val attempt = saveAttempts[captured.draft] ?: run {
            // This queued action runs after edits preceding the click and before
            // subsequent patches. Capture the service's acknowledged revision.
            val exact = if (entry != null) io { binding.session.editor().form }
                else io { binding.session.metadata(captured.draft).form }
            if (!current(binding) || exact.draft != captured.draft) return null
            SaveAttempt(exact, receipts.getOrPut(exact.draft to exact.revision, ::newOperation), clickedInput)
                .also { saveAttempts[exact.draft] = it }
        }
        saveLabel = SaveLabel.SAVING; invalidInput = false; error = false
        try {
            val saved = io { binding.session.save(attempt.form.draft, attempt.form.revision, attempt.operation) }
            if (!current(binding) || activeForm?.draft != captured.draft || saved.database != binding.database || saved.generation != binding.generation) return null
            // Refresh identity only. Keyboard input and focus belong to this existing UI form.
            val updated = if (entry != null) {
                val view = io { binding.session.editor() }
                if (!current(binding)) return null
                entry = entry?.copy(source = view); view.form
            } else {
                val view = io { binding.session.metadata(captured.draft) }
                if (!current(binding)) return null
                metadata = metadata?.copy(source = view); view.form
            }
            activeForm = updated
            saveAttempts.remove(captured.draft)
            if (captured.kind == FormKind.NEW_GROUP && saved.state == SaveState.SAVED) selectedGroup = updated.target
            saveLabel = if (inputRevision != attempt.input) SaveLabel.DIRTY else when (saved.state) {
                SaveState.SAVED -> SaveLabel.SAVED
                SaveState.UNCHANGED -> SaveLabel.CLEAN
            }
            error = false; refresh(binding)
            return SaveCompletion(updated, attempt.input)
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (failure: Exception) {
            if (binding.session.access() != SessionAccess.OPEN) {
                if (session === binding.session && epoch == binding.epoch) clearSensitive()
                return null
            }
            val invalid = failure is AndroidException.InvalidInput
            if (invalid) {
                saveAttempts.remove(captured.draft)
                receipts.remove(attempt.form.draft to attempt.form.revision)
            }
            val retained = try { io { binding.session.persistForms() }; true }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { false } // Preserve the original failed Save and keep its editor.
            if (current(binding)) { invalidInput = invalid; error = !invalid || !retained; saveLabel = SaveLabel.FAILED }
            return null
        }
    }
    fun showHistory(kind: FormKind, target: String?, revision: HistoryRow? = null, opened: (HistorySelection) -> Unit = {}) = navigate { binding ->
        val group = inspectedEntry?.takeIf { it.id == target }?.group
            ?: entry?.source?.takeIf { it.entry == target }?.group
        val rows = io { binding.session.history(kind, target) }
        if (current(binding)) {
            comparison.close(); metadataHistory.close(); clearEditor(); history = HistorySelection(kind, target, rows, group)
            historicalEntry = null; historicalRevision = null; historicalProperties = null; historicalTab = 0; revealed = ""
            revision?.takeIf { chosen -> rows.any { it.id == chosen.id } }?.let(::selectRevision)
            history?.let(opened)
        }
    }
    fun selectRevision(row: HistoryRow) {
        val selected = history ?: return
        if (selected.kind != FormKind.ENTRY) {
            hideSecrets(); metadataHistory.open(selected.kind, selected.target, row.id)
            return
        }
        metadataHistory.close()
        comparison.close()
        hideSecrets(); historicalEntry = null; historicalRevision = row.id; historicalProperties = null; historicalTab = 0
        historicalAttachments = emptyList(); historicalAttributes = emptyList()
        bound { binding ->
            val selection = history ?: return@bound
            if (selection.kind == FormKind.ENTRY && selection.target != null) {
                val view = io { binding.session.revision(selection.target, row.id) }
                val information = io { binding.session.revisionProperties(selection.target, row.id) }
                val rows = io { binding.session.entryAttachments(selection.target, row.id) }
                val attributes = io { binding.session.entryAttributes(selection.target, row.id) }
                if (current(binding) && historicalRevision == row.id) {
                    historicalEntry = view; historicalProperties = information
                    historicalAttachments = rows; historicalAttributes = attributes
                }
            }
        }
    }
    fun selectHistoricalTab(tab: Int) {
        if (tab !in 0..4 || historicalTab == tab) return
        hideSecrets(); historicalTab = tab
    }
    fun dismissHistory() {
        metadataHistory.close()
        comparison.close(); history = null; historicalEntry = null; historicalRevision = null
        historicalProperties = null; historicalTab = 0; historicalAttachments = emptyList(); historicalAttributes = emptyList(); hideSecrets()
    }
    private data class SecretTarget(val entry: String, val revision: String?, val attribute: String?, val saved: Boolean)
    private fun secretTarget(attribute: String?): SecretTarget? {
        val id = history?.takeIf { it.kind == FormKind.ENTRY }?.target ?: entry?.source?.entry ?: inspectedEntry?.id ?: return null
        return SecretTarget(id, historicalRevision, attribute, entry == null || entry?.source?.entry != null)
    }
    private fun accessBinding(binding: Binding) = AccessBinding(accessOwner, binding.epoch, binding.generation)
    private fun accessTicket(id: String, grant: SecretAccessGrant, permitted: () -> Boolean): SecretDisplayTicket? {
        val selected = binding() ?: run { grant.close(); return null }
        val request = secretEpoch
        val stamp = accessBinding(selected)
        return SecretDisplayTicket(grant, { current(selected) && secretEpoch == request && permitted() },
            { accesses.accept(AccessTarget(selected.database, id), stamp, it) })
    }
    fun reveal(attribute: String? = null) {
        val target = secretTarget(attribute) ?: return
        val request = secretEpoch
        val sequence = ++secretReadSequence
        bound { binding ->
            if (secretEpoch != request || secretReadSequence != sequence || secretTarget(attribute) != target) return@bound
            val granted = if (target.saved) ownedIo({ it.grant.close() }) { binding.session.revealForAccess(target.entry, target.revision, attribute) } else null
            val value = granted?.value ?: io { binding.session.reveal(target.entry, target.revision, attribute) }
            if (!current(binding) || secretEpoch != request || secretReadSequence != sequence || secretTarget(attribute) != target) {
                granted?.grant?.close(); return@bound
            }
            secretDisplay?.discard()
            val identity = Any()
            displayIdentity = identity
            secretDisplay = granted?.let { response -> accessTicket(target.entry, response.grant) {
                displayIdentity === identity && secretTarget(attribute) == target
            } }
            revealed = value; revealedAttribute = attribute
        }
    }
    fun copySecret(copy: (String) -> Boolean, attribute: String? = null) {
        val target = secretTarget(attribute) ?: return
        val request = secretEpoch
        val sequence = ++secretReadSequence
        bound { binding ->
            if (secretEpoch != request || secretReadSequence != sequence || secretTarget(attribute) != target) return@bound
            val granted = if (target.saved) ownedIo({ it.grant.close() }) { binding.session.revealForAccess(target.entry, target.revision, attribute) } else null
            val value = granted?.value ?: io { binding.session.reveal(target.entry, target.revision, attribute) }
            if (!current(binding) || secretEpoch != request || secretReadSequence != sequence || secretTarget(attribute) != target) {
                granted?.grant?.close(); return@bound
            }
            val ticket = granted?.let { response -> accessTicket(target.entry, response.grant) { secretTarget(attribute) == target } }
            try { if (copy(value)) { if (ticket?.acceptCopy() == false) ticket.discard() } else ticket?.discard() }
            catch (failure: Exception) { ticket?.discard(); throw failure }
        }
    }
    fun hideSecrets() {
        secretEpoch++; secretReadSequence++; displayIdentity = null; secretDisplay?.discard(); secretDisplay = null
        revealed = ""; revealedAttribute = null; invitation = ""; received.hideSecret(); comparison.hideSecrets()
    }
    fun purgeHistory(selection: HistorySelection, revisions: List<String>, operation: String, finished: (Boolean) -> Unit = {}) {
        if (binding() == null) { finished(false); return }
        error = false
        bound { binding ->
            try { io { binding.session.purgeHistory(selection.kind, selection.target, revisions, operation) } }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (current(binding)) { error = true; finished(false) }; return@bound }
            if (current(binding)) {
                comparison.close(); historicalEntry = null; historicalRevision = null; historicalProperties = null; historicalTab = 0
                historicalAttachments = emptyList(); historicalAttributes = emptyList(); hideSecrets()
                finished(true)
                // Purge is already durably acknowledged; a later projection failure cannot undo it.
                history = HistorySelection(selection.kind, selection.target, io { binding.session.history(selection.kind, selection.target) }, selection.group)
            }
        }
    }
    fun restoreRevision(entry: String, revision: String, operation: String, destination: String? = null, finished: (Boolean) -> Unit = {}) {
        if (editingBlocked || binding() == null) { finished(false); return }
        error = false
        val group = destination ?: inspectedEntry?.group ?: this.entry?.source?.group ?: historicalEntry?.group ?: history?.group
        if (group == null) { error = true; finished(false); return }
        restorations.confirm(entry, revision, group, operation, finished)
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
        leaveEditor(::openExchangeAfterNavigation)
    }
    internal fun openExchangeAfterNavigation() {
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
    fun join(code: String, password: String) = startExchange { request ->
        val owner = manager.await()
        io { owner.join(code, password) }
        val copies = io { owner.catalog() }; val requests = io { owner.pendingJoins() }
        if (epoch == request) { catalog = copies; pending = requests }
    }
    fun resumeJoin(request: String, password: String = "") = startExchange { requestedEpoch ->
        val owner = manager.await()
        io { owner.resumeJoin(request, password) }
        val copies = io { owner.catalog() }; val requests = io { owner.pendingJoins() }
        if (epoch == requestedEpoch) { catalog = copies; pending = requests }
    }
    internal fun showPolicy(page: PolicyPage = PolicyPage.PROTECTION) = navigate { binding ->
        if (current(binding)) { clearEditor(); policySettings.show(page) }
    }
    fun dismissPolicy() { policySettings.close() }
    fun rotate(password: String, revoke: String?, operation: String, finished: (RotationOutcome) -> Unit = {}) {
        if (binding() == null) { finished(RotationOutcome.REJECTED); return }
        error = false
        bound { binding ->
            try {
                io { binding.session.rotatePassword(password, revoke, operation) }
                if (session === binding.session && epoch == binding.epoch) { finished(RotationOutcome.CONFIRMED); clearSensitive() }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (failure: Exception) {
                if (session === binding.session && epoch == binding.epoch) {
                    if (binding.session.access() != SessionAccess.OPEN) clearSensitive()
                    else {
                        error = true
                        finished(if (failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions)
                            RotationOutcome.REJECTED else RotationOutcome.UNCERTAIN)
                    }
                }
            }
        }
    }
    fun clearError() { error = false }
    fun reportFailure() { error = true }
    fun activity() { manager.getCompletedOrNull()?.activity() }
    fun lock() {
        try {
            session?.lock()
            manager.getCompletedOrNull()?.background()
        } finally {
            // Revoke local values in the same UI action, independently of the status poller.
            clearSensitive()
        }
    }
    /** Manual UI lock is scoped to the selected worker; OS background still calls lock(). */
    internal fun lockCurrent() {
        try { session?.lock() } finally { clearSelectedSensitive() }
    }
    private fun clearEditor() {
        clearIconPreviews()
        entry = null; inspectedEntry = null; metadata = null; activeForm = null
        appearance = null; properties = null; entryHistory = emptyList()
        inspectedAttributes = emptyList(); attachments = emptyList()
        historicalAttachments = emptyList(); historicalAttributes = emptyList()
        historicalProperties = null; historicalTab = 0
        retryEdits.clear(); hideSecrets(); inputRevision++; appliedInputRevision = inputRevision
        saveLabel = SaveLabel.CLEAN; invalidInput = false
    }
    private fun clearContent() {
        biometry.clear()
        management.close()
        policySettings.reset()
        groupFavicons.reset()
        clearEditor(); lifecycle.reset(); received.reset(); comparison.close(); restorations.reset(); metadataHistory.reset(); applicationStatus = null
        observedApplicationBinding = null; observedApplicationRevision = null
        observedAppliedRevision = 0uL; incomingRefreshQueued = false
        overview = null; forms = emptyList(); history = null; historicalEntry = null; historicalRevision = null
        undo = null; selectedGroup = null; hideSecrets()
    }
    private fun clearSelectedSensitive() {
        overview?.database?.let { id -> lockedCopy = catalog.firstOrNull { it.database == id } }
        icons.invalidateBinding(); recovery.lock(); management.lock(); epoch++; secrecyEpoch++; pendingNavigation = null; navigationDecision = false
        clearContent(); opening = false; navigating = false; error = false
        val clearedEpoch = epoch
        val old = session; session = null
        while (commands.tryReceive().isSuccess) { /* Drop pending entered JVM values immediately. */ }
        if (old != null) viewModelScope.launch {
            try { io { manager.awaitBlocking().close(old) } }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == clearedEpoch && session == null) error = true }
        }
        searches.reconcile()
    }
    private fun clearSensitive() {
        searches.close(); unlockedCopies = emptyList(); sourceGroups.clear()
        clearSelectedSensitive()
        manager.getCompletedOrNull()?.let { owner -> viewModelScope.launch {
            try { io { owner.closeRevoked() } }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { /* The supervisor retains actual worker shutdown status. */ }
        } }
    }
    override fun onCleared() {
        groupFavicons.close()
        databaseCopy.cancel(); copySource = null
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
