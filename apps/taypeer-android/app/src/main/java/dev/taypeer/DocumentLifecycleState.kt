package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.*

/** Executes through DocumentState's existing session binding and guarded navigation. */
internal interface SessionCommands {
    fun <T> execute(navigate: Boolean, command: (DocumentSession) -> T, started: () -> Unit,
        completed: (T) -> Unit, failed: () -> Unit, rejected: () -> Unit = failed, discarded: (T) -> Unit = {}): Boolean
}

/** Exact submitted intent. Ordinary clone text stays in this VM, never saved or logged. */
class ObjectActionRequest internal constructor(val database: String, val kind: FormKind,
    val target: String, val action: ObjectAction, val destination: String?, val name: String,
    val operation: String)

private data class ObjectActionAddress(val database: String, val kind: FormKind,
    val target: String, val action: ObjectAction)

/** Presentation only. Exact lifecycle selections and durable receipts remain inside Rust. */
class DocumentLifecycleState internal constructor(private val commands: SessionCommands,
    private val database: () -> String?, private val selectDestination: (String?) -> Unit) {
    var showingTrash by mutableStateOf(false); private set
    var rows by mutableStateOf<List<TrashRow>>(emptyList()); private set
    var review by mutableStateOf<TrashReviewUi?>(null); private set
    var selection by mutableStateOf<ObjectActionSelection?>(null); private set
    var attempt by mutableStateOf<ObjectActionRequest?>(null); private set
    var busy by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    private var waitingForNavigation = false
    private var epoch = 0L
    // A route departure or lock cannot turn an uncertain clone into a new operation.
    private var requests by mutableStateOf<Map<ObjectActionAddress, ObjectActionRequest>>(emptyMap())
    val unresolvedActions: List<ObjectActionRequest>
        get() = requests.filterKeys { it.database == database() }.values.toList()

    private fun address(value: ObjectActionSelection): ObjectActionAddress? =
        database()?.let { ObjectActionAddress(it, value.kind, value.target, value.action) }
    private fun address(value: ObjectActionRequest) =
        ObjectActionAddress(value.database, value.kind, value.target, value.action)

    private fun <T> run(navigate: Boolean = false, command: (DocumentSession) -> T,
        rejected: () -> Unit = {}, completed: (T) -> Unit) {
        if (busy) return
        val expectedEpoch = epoch
        val expectedDatabase = database()
        fun current() = epoch == expectedEpoch && database() == expectedDatabase
        busy = true; error = false; waitingForNavigation = navigate
        val accepted = commands.execute(navigate, command,
            started = { if (current()) { waitingForNavigation = false; busy = true } },
            completed = { value -> if (current()) { busy = false; completed(value) } },
            failed = { if (current()) { busy = false; waitingForNavigation = false; error = true } },
            rejected = { if (current()) { busy = false; waitingForNavigation = false; error = true; rejected() } })
        if (!accepted && current()) { busy = false; waitingForNavigation = false }
    }

    fun openTrash() = run(navigate = true, command = { it.trashRows() }) {
        rows = it; showingTrash = true; selection = null; attempt = null
    }

    fun refreshTrash() = run(command = { it.trashRows() }) { rows = it }

    fun prepareRestore(row: TrashRow, destination: String?) = prepare(row, restoring = true, destination)
    fun preparePurge(row: TrashRow) = prepare(row, restoring = false, null)
    private fun prepare(row: TrashRow, restoring: Boolean, destination: String?) {
        if (review != null) return
        run(command = { session ->
            if (restoring) session.prepareTrashRestore(row.kind, row.target, row.generation, destination)
            else session.prepareTrashPurge(row.kind, row.target, row.generation)
        }) { prepared -> review = TrashReviewUi(row, prepared, restoring, newOperation()) }
    }

    fun confirmReview() {
        val retained = review ?: return
        run(command = { it.confirmTrashReview(retained.review.review, retained.operation) }) {
            // A durable confirmation is complete even if refreshing the projection later fails.
            review = null
            cleanupReview(retained, close = false)
        }
    }

    fun cancelReview() { review?.let { cleanupReview(it, close = false) } }
    private fun cleanupReview(retained: TrashReviewUi, close: Boolean) = run(command = {
        it.discardTrashReview(retained.review.review)
        it.trashRows()
    }) { refreshed ->
        review = null; rows = refreshed
        if (close) showingTrash = false
    }

    fun closeTrash() {
        if (busy) return
        val retained = review
        if (retained != null) cleanupReview(retained, close = true)
        else { showingTrash = false; rows = emptyList(); error = false }
    }

    fun openAction(value: ObjectActionSelection) = run(navigate = true, command = { Unit }) {
        val retained = address(value)?.let(requests::get)
        selection = if (retained == null) value else value.copy(name = retained.name, destination = retained.destination)
        attempt = retained; showingTrash = false
    }

    fun confirmAction(destination: String?, name: String, operation: String) {
        val selected = selection ?: return
        if (busy) return
        val key = address(selected) ?: return
        if (requests.size >= 256 && key !in requests) { error = true; return }
        val request = requests[key] ?: ObjectActionRequest(key.database, selected.kind, selected.target,
            selected.action, destination, name, operation).also { requests = requests + (key to it) }
        performAction(key, request)
    }

    /** Reconcile the exact retained operation even if the source no longer exists. */
    fun retry(request: ObjectActionRequest) {
        if (busy || request.database != database()) return
        val key = address(request)
        if (requests[key] !== request) return
        performAction(key, request)
    }

    private fun performAction(key: ObjectActionAddress, request: ObjectActionRequest) {
        attempt = request
        run(rejected = {
            if (requests[key] === request) requests = requests - key
            if (attempt === request) attempt = null
        }, command = { session ->
            when {
                request.kind == FormKind.ENTRY && request.action == ObjectAction.CLONE ->
                    session.cloneEntry(request.target, request.destination ?: throw AndroidException.InvalidInput(), request.name, request.operation)
                request.kind == FormKind.ENTRY -> {
                    session.moveEntry(request.target, request.destination ?: throw AndroidException.InvalidInput(), request.operation)
                    request.target
                }
                request.action == ObjectAction.CLONE -> {
                    session.cloneGroup(request.target, request.destination, request.name, request.operation)
                    request.target
                }
                else -> {
                    session.moveGroup(request.target, request.destination, request.operation)
                    request.target
                }
            }
        }) {
            // The shared command's durable reply resolves this intent before any UI refresh.
            requests = requests - key
            selectDestination(request.destination); selection = null; attempt = null
        }
    }

    fun closeAction() { if (!busy) { selection = null; attempt = null; error = false } }

    internal fun navigationStayed() {
        if (waitingForNavigation) { waitingForNavigation = false; busy = false }
    }

    internal fun reset() {
        epoch++
        showingTrash = false; rows = emptyList(); review = null; selection = null; attempt = null
        busy = false; error = false; waitingForNavigation = false
    }
}
