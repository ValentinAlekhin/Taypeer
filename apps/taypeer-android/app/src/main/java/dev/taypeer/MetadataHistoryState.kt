package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.*

internal enum class MetadataHistoryMode { SAVED, COMPARE, RESTORE }
internal data class MetadataVersionSelection(val database: String, val kind: FormKind,
    val target: String?, val revision: String)

/** Only public exact intent survives Back/lock. Snapshot values and PNGs never enter this ledger. */
class MetadataRestoreRequest internal constructor(val database: String, val kind: FormKind,
    val target: String?, val revision: String, val parent: String?, val basis: List<String>, val operation: String) {
    internal var submitted = false
}

private data class MetadataRestoreAddress(val database: String, val kind: FormKind,
    val target: String?, val revision: String)

/** Rust owns history, destination validation and durability. This owner only fences UI delivery. */
internal class MetadataHistoryState(private val commands: SessionCommands,
    private val database: () -> String?, private val restored: (FormKind, String) -> Unit) {
    var selection by mutableStateOf<MetadataVersionSelection?>(null); private set
    var showing by mutableStateOf(false); private set
    var mode by mutableStateOf(MetadataHistoryMode.SAVED); private set
    var view by mutableStateOf<MetadataRevisionView?>(null); private set
    var savedPreview by mutableStateOf<RenderedIcon?>(null); private set
    var currentPreview by mutableStateOf<RenderedIcon?>(null); private set
    var busy by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    private var requests by mutableStateOf<Map<MetadataRestoreAddress, MetadataRestoreRequest>>(emptyMap())
    private var epoch = 0L
    private var readRevision = 0L
    private var refreshQueued = false
    private var waitingForNavigation = false
    private var restoring: MetadataRestoreRequest? = null
    val unresolvedRequests get() = requests.filterKeys { it.database == database() }.values.toList()

    private fun address(value: MetadataVersionSelection) = MetadataRestoreAddress(value.database, value.kind, value.target, value.revision)
    private fun address(value: MetadataRestoreRequest) = MetadataRestoreAddress(value.database, value.kind, value.target, value.revision)
    fun request(): MetadataRestoreRequest? = selection?.let { requests[address(it)] }

    fun open(kind: FormKind, target: String?, revision: String, mode: MetadataHistoryMode = MetadataHistoryMode.SAVED) {
        if (busy || (kind != FormKind.GROUP && kind != FormKind.DATABASE)) return
        val selected = MetadataVersionSelection(database() ?: return, kind, target, revision)
        selection = selected; showing = false; this.mode = mode; readRevision++; clearProjection(); error = false; refreshQueued = false
        read(selected, navigate = true)
    }

    fun compare() { if (!busy && view != null) mode = MetadataHistoryMode.COMPARE }
    fun prepareRestore() { if (!busy && view != null) mode = MetadataHistoryMode.RESTORE }
    fun showSaved() { if (!busy) mode = MetadataHistoryMode.SAVED }

    /** A received projection invalidates the displayed comparison and all in-flight thumbnails. */
    fun refreshIfOpen() {
        val selected = selection ?: return
        readRevision++; clearProjection()
        if (busy) { refreshQueued = true; return }
        read(selected, navigate = false)
    }
    fun retryRead() { if (!busy) selection?.let { read(it, navigate = false) } }

    private class Projection(val value: MetadataRevisionView, val saved: RenderedIcon?, val current: RenderedIcon?) {
        fun clear() { saved?.png?.fill(0); current?.png?.fill(0) }
    }
    private fun projection(session: DocumentSession, selected: MetadataVersionSelection): Projection {
        val value = session.metadataRevision(selected.kind, selected.target, selected.revision)
        if (selected.kind != FormKind.GROUP) return Projection(value, null, null)
        val group = selected.target ?: throw AndroidException.InvalidOptions()
        var saved: RenderedIcon? = null
        var current: RenderedIcon? = null
        try {
            if (value.saved.icon == IconView.Image) saved = session.iconPreview(IconScope.GroupRevision(group, selected.revision))
            if (value.current.icon == IconView.Image) current = session.iconPreview(IconScope.Group(group))
            return Projection(value, saved, current)
        } catch (failure: Throwable) { saved?.png?.fill(0); current?.png?.fill(0); throw failure }
    }

    private fun read(selected: MetadataVersionSelection, navigate: Boolean) {
        if (busy || selection != selected || database() != selected.database) return
        val expectedEpoch = epoch
        val expectedRead = readRevision
        fun current() = expectedEpoch == epoch && selection == selected && database() == selected.database
        fun finished() {
            if (!current()) return
            busy = false; waitingForNavigation = false
            if (refreshQueued) { refreshQueued = false; read(selected, navigate = false) }
        }
        busy = true; error = false; waitingForNavigation = navigate
        val accepted = commands.execute(navigate, { projection(it, selected) },
            started = { if (current()) { waitingForNavigation = false; showing = true } },
            completed = { result ->
                if (current() && readRevision == expectedRead) {
                    clearProjection(); view = result.value; savedPreview = result.saved; currentPreview = result.current
                } else result.clear()
                finished()
            },
            failed = { if (current()) { error = true; finished() } },
            discarded = { it.clear() })
        if (!accepted && current()) { busy = false; waitingForNavigation = false }
    }

    fun confirm(parent: String?, operation: String, finished: (Boolean) -> Unit = {}) {
        if (busy) return
        val selected = selection ?: return
        val key = address(selected)
        val retained = requests[key] ?: run {
            val projected = view ?: return
            if (requests.size >= 256) { error = true; return }
            MetadataRestoreRequest(selected.database, selected.kind, selected.target, selected.revision,
                if (selected.kind == FormKind.GROUP) parent else null, projected.basis.toList(), operation)
                .also { requests = requests + (key to it) }
        }
        retry(retained, finished)
    }

    /** Replay bypasses source lookup: a committed source may have since been purged. */
    fun retry(request: MetadataRestoreRequest, finished: (Boolean) -> Unit = {}) {
        val key = address(request)
        if (busy || request.database != database() || requests[key] !== request) return
        val expectedEpoch = epoch
        fun current() = epoch == expectedEpoch && database() == request.database
        var acknowledged = false
        busy = true; error = false; waitingForNavigation = true; restoring = request
        val accepted = commands.execute(navigate = true, command = { session ->
            when (request.kind) {
                FormKind.GROUP -> session.restoreGroupRevision(request.target ?: throw AndroidException.InvalidOptions(),
                    request.revision, request.parent, request.basis, request.operation)
                FormKind.DATABASE -> session.restoreDatabaseRevision(request.revision, request.basis, request.operation)
                else -> throw AndroidException.InvalidOptions()
            }
        }, started = { request.submitted = true; if (current()) waitingForNavigation = false },
            completed = { id -> if (current()) {
                // This ACK resolves the mutation even if subsequent projection/navigation fails.
                acknowledged = true; requests = requests - key
                busy = false; waitingForNavigation = false; restoring = null; refreshQueued = false
                selection = null; showing = false; readRevision++; clearProjection()
                finished(true); restored(request.kind, id)
            } }, failed = { if (current()) {
                busy = false; waitingForNavigation = false; restoring = null; error = true
                if (!acknowledged) finished(false)
            } }, rejected = { if (current()) {
                busy = false; waitingForNavigation = false; restoring = null; error = true
                if (!acknowledged) { if (requests[key] === request) requests = requests - key; finished(false) }
            } })
        if (!accepted && current()) {
            if (!request.submitted && requests[key] === request) requests = requests - key
            busy = false; waitingForNavigation = false; restoring = null; finished(false)
        }
    }

    fun close() {
        if (busy) return
        epoch++; readRevision++; selection = null; showing = false; clearProjection(); error = false; refreshQueued = false
    }
    private fun clearProjection() {
        savedPreview?.png?.fill(0); currentPreview?.png?.fill(0)
        savedPreview = null; currentPreview = null; view = null
    }
    internal fun navigationStayed() {
        if (!waitingForNavigation) return
        // started never ran: this intent was not sent to a worker and is safe to replace.
        restoring?.let { request -> if (!request.submitted && requests[address(request)] === request) requests = requests - address(request) }
        waitingForNavigation = false; restoring = null; busy = false
        close()
    }
    internal fun reset() {
        epoch++; readRevision++; selection = null; showing = false; clearProjection()
        busy = false; error = false; waitingForNavigation = false; restoring = null; refreshQueued = false
    }
}
