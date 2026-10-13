package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.DocumentSession

/** Exact public identifiers only. Never persisted, rendered while locked, or logged. */
class RestoreRequest internal constructor(val database: String, val entry: String,
    val revision: String, val destination: String, val operation: String)

private data class RestoreAddress(val database: String, val entry: String, val revision: String)

/** Retains uncertain UI intent; shared Rust owns restoration and durable replay receipts. */
class RestoreRevisionState internal constructor(private val commands: SessionCommands,
    private val database: () -> String?, private val openRestored: (String) -> Unit) {
    private var requests by mutableStateOf<Map<RestoreAddress, RestoreRequest>>(emptyMap())
    private var epoch = 0L
    private var waitingForNavigation = false
    var busy by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    val unresolvedRequests get() = requests.filterKeys { it.database == database() }.values.toList()

    fun request(entry: String, revision: String): RestoreRequest? =
        database()?.let { requests[RestoreAddress(it, entry, revision)] }

    fun confirm(entry: String, revision: String, destination: String, operation: String,
        finished: (Boolean) -> Unit = {}) {
        if (busy) return
        val currentDatabase = database() ?: return
        val key = RestoreAddress(currentDatabase, entry, revision)
        if (requests.size >= 256 && key !in requests) { error = true; finished(false); return }
        val retained = requests[key] ?: RestoreRequest(currentDatabase, entry, revision, destination, operation)
            .also { requests = requests + (key to it) }
        retry(retained, finished)
    }

    /** Does not inspect the source first: the source can have been purged since commit. */
    fun retry(request: RestoreRequest, finished: (Boolean) -> Unit = {}) {
        val key = RestoreAddress(request.database, request.entry, request.revision)
        if (busy || request.database != database() || requests[key] !== request) return
        val expectedEpoch = epoch
        var acknowledged = false
        fun current() = epoch == expectedEpoch && database() == request.database
        busy = true; error = false; waitingForNavigation = true
        val accepted = commands.execute(navigate = true,
            command = { session: DocumentSession -> session.restoreRevision(request.entry,
                request.revision, request.destination, request.operation) },
            started = { if (current()) { busy = true; waitingForNavigation = false } },
            completed = { restored -> if (current()) {
                // Receipt is acknowledged before projection/navigation, which can fail independently.
                acknowledged = true; requests = requests - key; busy = false; waitingForNavigation = false
                finished(true); openRestored(restored)
            } },
            failed = { if (current()) { busy = false; waitingForNavigation = false; error = true; if (!acknowledged) finished(false) } },
            rejected = { if (current()) {
                busy = false; waitingForNavigation = false; error = true
                if (!acknowledged) {
                    if (requests[key] === request) requests = requests - key
                    finished(false)
                }
            } })
        if (!accepted && current()) { busy = false; waitingForNavigation = false; finished(false) }
    }

    internal fun navigationStayed() {
        if (waitingForNavigation) { waitingForNavigation = false; busy = false }
    }
    internal fun reset() { epoch++; busy = false; error = false; waitingForNavigation = false }
}
