package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.*

/** Distinct shared dispositions; actor IDs do not imply current network membership. */
sealed interface SourceRow {
    val id: String
    val author: String
    data class Received(val value: ReceivedSourceRow) : SourceRow {
        override val id get() = value.change
        override val author get() = value.author
    }
    data class Closed(val value: PendingSourceRow) : SourceRow {
        override val id get() = value.source
        override val author get() = value.actor
    }
}

data class ReceivedExtraction(val source: SourceRow, val entry: String?, val group: String?, val operation: String)
private data class SourceAddress(val database: String, val closed: Boolean, val source: String, val entry: String?)
private data class SourceInspection(val entries: List<EntryRow>, val group: GroupRow?, val passwords: List<PendingPasswordVariant>)
private data class SourceExtractionResult(val request: ReceivedExtraction, val objectView: ChangedObject)

/** Rust owns disposition and durable actions. Retained retry IDs contain no source values. */
class ReceivedSourcesState internal constructor(private val commands: SessionCommands, private val database: () -> String?) {
    var showing by mutableStateOf(false); private set
    var rows by mutableStateOf<List<SourceRow>>(emptyList()); private set
    var selected by mutableStateOf<SourceRow?>(null); private set
    var entries by mutableStateOf<List<EntryRow>>(emptyList()); private set
    var entry by mutableStateOf<EntryRow?>(null); private set
    var group by mutableStateOf<GroupRow?>(null); private set
    var passwordVariants by mutableStateOf<List<PendingPasswordVariant>>(emptyList()); private set
    var passwordVariant by mutableStateOf<PendingPasswordVariant?>(null); private set
    var revealed by mutableStateOf<String?>(null); private set
    var busy by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    var attempt by mutableStateOf<ReceivedExtraction?>(null); private set
    var extracted by mutableStateOf<ChangedObject?>(null); private set
    private var waitingForNavigation = false
    private var secretEpoch = 0L
    private var epoch = 0L
    // Route departure and lock clear inspected values, but cannot forget an uncertain intent.
    private val requests = linkedMapOf<SourceAddress, ReceivedExtraction>()
    private val confirmations = linkedMapOf<SourceAddress, SourceExtractionResult>()
    val unresolvedRequests: List<ReceivedExtraction>
        get() = requests.filterKeys { it.database == database() }.values.toList()

    private fun address(source: SourceRow, entry: String? = this.entry?.id): SourceAddress? =
        database()?.let { SourceAddress(it, source is SourceRow.Closed, source.id, entry) }

    private fun <T> run(navigate: Boolean = false, command: (DocumentSession) -> T,
        rejected: () -> Unit = {}, completed: (T) -> Unit) {
        if (busy) return
        val expectedEpoch = epoch
        val expectedDatabase = database()
        fun current() = epoch == expectedEpoch && database() == expectedDatabase
        busy = true; error = false; waitingForNavigation = navigate
        val accepted = commands.execute(navigate, command,
            started = { if (current()) { waitingForNavigation = false; busy = true } },
            completed = { result -> if (current()) { busy = false; completed(result) } },
            failed = { if (current()) { busy = false; waitingForNavigation = false; error = true } },
            rejected = { if (current()) { busy = false; waitingForNavigation = false; error = true; rejected() } })
        if (!accepted && current()) { busy = false; waitingForNavigation = false }
    }
    private fun sourceRows(session: DocumentSession): List<SourceRow> =
        session.pendingSources().map(SourceRow::Closed) +
            session.receivedSources().filterNot(ReceivedSourceRow::discarded).map(SourceRow::Received)
    private fun retainRequests(updated: List<SourceRow>): List<SourceRow> {
        val current = database()
        val unresolved = requests.filterKeys { it.database == current }.values.map { it.source }
        return (updated + unresolved).distinctBy { Pair(it is SourceRow.Closed, it.id) }
    }
    fun open() = run(navigate = true, command = ::sourceRows) {
        resetSelection(); rows = retainRequests(it); showing = true
    }
    fun refresh() = run(command = ::sourceRows) { updated ->
        rows = retainRequests(updated)
        // Processing removes the native pending row; retain its confirmed result until the user leaves.
        if (selected != null && extracted == null && rows.none { it::class == selected!!::class && it.id == selected?.id }) resetSelection()
    }
    internal fun refreshIfOpen() { if (showing && !busy) refresh() }

    fun inspect(row: SourceRow) {
        if (busy) return
        hideSecret()
        run(command = { session -> when (row) {
            is SourceRow.Received -> SourceInspection(session.inspectReceived(row.value.change), null, emptyList())
            is SourceRow.Closed -> session.inspectPending(row.value.source).let {
                SourceInspection(listOfNotNull(it.entry), it.group, it.passwordVariants)
            }
        } }) {
            selected = row; entries = it.entries; entry = it.entries.firstOrNull(); group = it.group
            passwordVariants = it.passwords; passwordVariant = it.passwords.singleOrNull(); attachRequest()
        }
    }
    private fun attachRequest() {
        val selected = selected ?: return
        val key = address(selected) ?: return
        attempt = requests[key]
        extracted = confirmations[key]?.objectView
    }
    fun selectEntry(row: EntryRow) {
        if (busy) return
        hideSecret(); entry = row; attachRequest()
    }
    fun selectPassword(variant: PendingPasswordVariant) {
        if (busy || variant !in passwordVariants) return
        hideSecret(); passwordVariant = variant
    }
    private fun revealCommand(session: DocumentSession, source: SourceRow, chosen: EntryRow, variant: PendingPasswordVariant?): String = when (source) {
        is SourceRow.Received -> session.revealReceived(source.value.change, chosen.id)
        is SourceRow.Closed -> session.revealPending(source.value.source, variant?.origins ?: throw AndroidException.InvalidInput())
    }
    private fun readPassword(complete: (String) -> Unit) {
        val source = selected ?: return
        val chosen = entry ?: return
        val variant = passwordVariant
        val expected = secretEpoch
        run(command = { revealCommand(it, source, chosen, variant) }) {
            if (secretEpoch == expected && selected == source && entry?.id == chosen.id && passwordVariant == variant) complete(it)
        }
    }
    fun reveal() { if (revealed != null) hideSecret() else readPassword { revealed = it } }
    fun copyPassword(copy: (String) -> Unit) = readPassword(copy)

    fun extract(destination: String?) {
        if (busy || extracted != null) return
        val source = selected ?: return
        val key = address(source) ?: return
        if (requests.size >= 256 && key !in requests) { error = true; return }
        val request = requests[key] ?: ReceivedExtraction(source, entry?.id, destination, newOperation()).also { requests[key] = it }
        performExtraction(key, request)
    }
    fun retryExtraction(request: ReceivedExtraction) {
        if (busy) return
        val key = address(request.source, request.entry) ?: return
        if (requests[key] != request) return
        performExtraction(key, request)
    }
    private fun performExtraction(key: SourceAddress, request: ReceivedExtraction) {
        attempt = request; hideSecret()
        run(rejected = {
            if (requests[key] === request) requests.remove(key)
            if (attempt === request) attempt = null
        }, command = { session -> when (val retained = request.source) {
            is SourceRow.Received -> ChangedObject(FormKind.ENTRY, session.extractReceived(retained.value.change,
                request.entry ?: throw AndroidException.InvalidInput(), request.group ?: throw AndroidException.InvalidInput(), request.operation))
            is SourceRow.Closed -> session.recoverPending(retained.value.source, request.group, request.operation)
        } }) {
            requests.remove(key); confirmations[key] = SourceExtractionResult(request, it)
            while (confirmations.size > 256) confirmations.remove(confirmations.keys.first())
            extracted = it; attempt = null
        }
    }
    fun discard() {
        if (busy) return
        val source = selected as? SourceRow.Received ?: return
        // Resolve an uncertain extraction with its retained operation before another mutation.
        if (attempt != null) { error = true; return }
        hideSecret()
        run(command = { it.discardReceived(source.value.change); sourceRows(it) }) {
            rows = retainRequests(it); resetSelection()
        }
    }
    fun back() {
        if (busy) return
        if (selected != null) resetSelection() else { showing = false; rows = emptyList(); error = false }
    }
    fun close() { if (!busy) { resetSelection(); showing = false; rows = emptyList(); error = false } }
    fun hideSecret() { secretEpoch++; revealed = null }
    private fun resetSelection() {
        hideSecret(); selected = null; entries = emptyList(); entry = null; group = null
        passwordVariants = emptyList(); passwordVariant = null; attempt = null; extracted = null
    }
    internal fun navigationStayed() { if (waitingForNavigation) { busy = false; waitingForNavigation = false } }
    internal fun reset() {
        epoch++
        resetSelection(); showing = false; rows = emptyList(); busy = false; error = false; waitingForNavigation = false
    }
}
