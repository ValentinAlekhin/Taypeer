package dev.taypeer

import androidx.compose.runtime.*
import dev.taypeer.bridge.*
import kotlinx.coroutines.*
import java.security.SecureRandom

internal data class ManagementUiSession(val session: DocumentSession, val database: String,
    val generation: ULong, val epoch: Long)
internal sealed interface ManagementRoute {
    data class Transfer(val database: String, val operation: String, val recipient: String?) : ManagementRoute
    data class Decision(val intent: ManagementIntentView) : ManagementRoute
}
internal enum class ManagementProblem { UNCONFIRMED, NEEDS_UNLOCK, PENDING_APPLY, STALE, WRONG_ROLE, UNKNOWN, CONFLICT, CAPACITY, UNSUPPORTED }
private sealed interface ManagementRequest {
    val database: String
    val operation: String
    data class Propose(override val database: String, val recipient: String, override val operation: String) : ManagementRequest
    data class Decision(val intent: ManagementIntentView, val accept: Boolean) : ManagementRequest {
        override val database get() = intent.database
        override val operation get() = intent.operation
    }
    data class Retry(val intent: ManagementIntentView) : ManagementRequest {
        override val database get() = intent.database
        override val operation get() = intent.operation
    }
}

/** Public immutable requests survive routes and lock; Rust owns decisions, readiness and roles. */
internal class ManagementState(
    private val scope: CoroutineScope,
    private val host: suspend () -> Host,
    private val session: () -> ManagementUiSession?,
    private val current: (ManagementUiSession) -> Boolean,
    private val changed: (ManagementUiSession) -> Unit,
) {
    private data class Key(val database: String, val operation: String)
    private val retained = mutableStateMapOf<Key, ManagementRequest>()
    private val working = mutableStateMapOf<Key, Boolean>()
    private val problems = mutableStateMapOf<Key, ManagementProblem>()
    private var epoch = 0L
    private var routeEpoch = 0L
    private var projectionEpoch = 0L
    private var projectionSequence = 0L
    private var appliedProjection = 0L
    private var loading = false
    var database by mutableStateOf<String?>(null); private set
    var route by mutableStateOf<ManagementRoute?>(null); private set
    var rows by mutableStateOf<List<ManagementHandoffView>>(emptyList()); private set
    var peers by mutableStateOf<List<DeviceRow>>(emptyList()); private set
    var localDevice by mutableStateOf<String?>(null); private set
    var loaded by mutableStateOf(false); private set
    var loadFailed by mutableStateOf(false); private set
    var observation by mutableStateOf<ManagementPeerStatusView?>(null); private set
    var observing by mutableStateOf(false); private set
    var observationFailed by mutableStateOf(false); private set
    var preparing by mutableStateOf(false); private set
    val canDecide get() = session()?.let { it.database == database && current(it) } == true

    fun selectDatabase(value: String?) {
        if (value == database) return
        epoch++; database = value; rows = emptyList(); peers = emptyList(); localDevice = null
        loaded = false; loadFailed = false; loading = false; close()
        if (value != null) load()
    }
    fun load() {
        val selected = database ?: return
        if (loading) return
        val expected = epoch; val projection = projectionEpoch; val sequence = ++projectionSequence
        loading = true
        scope.launch {
            try {
                val owner = host()
                val (saved, devices) = withContext(Dispatchers.IO) { owner.managementHandoffs(selected) to owner.exchangeView(selected).devices }
                if (epoch != expected || database != selected || projection != projectionEpoch || sequence < appliedProjection) return@launch
                applyProjection(saved, devices, sequence)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == expected && projection == projectionEpoch && sequence >= appliedProjection) { loaded = true; loadFailed = true } }
            finally { if (epoch == expected) loading = false }
        }
    }

    fun openTransfer(recipient: String? = null) {
        val selected = database ?: return
        val binding = session()?.takeIf { it.database == selected && current(it) } ?: return
        val expected = epoch; val display = ++routeEpoch
        val projection = projectionEpoch; val sequence = ++projectionSequence
        clearObservation(); preparing = true
        scope.launch {
            try {
                val owner = host()
                val (saved, devices) = withContext(Dispatchers.IO) { owner.managementHandoffs(selected) to owner.exchangeView(selected).devices }
                if (epoch != expected || routeEpoch != display || !current(binding) || projection != projectionEpoch) return@launch
                if (sequence >= appliedProjection) applyProjection(saved, devices, sequence)
                val unfinished = rows.firstOrNull { it.intent.manager == localDevice && !terminal(it.phase) }
                val previous = retained.values.filterIsInstance<ManagementRequest.Propose>().firstOrNull { it.database == selected }
                route = when {
                    unfinished != null -> ManagementRoute.Transfer(selected, unfinished.intent.operation, unfinished.intent.recipient)
                    previous != null -> ManagementRoute.Transfer(selected, previous.operation, previous.recipient)
                    else -> ManagementRoute.Transfer(selected, newManagementOperation(), recipient?.takeIf { id -> peers.any { it.id == id } })
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == expected && routeEpoch == display) loadFailed = true }
            finally { if (epoch == expected && routeEpoch == display) preparing = false }
        }
    }
    fun openDecision(row: ManagementHandoffView) {
        if (row.intent.database != database || row.intent.recipient != localDevice) return
        routeEpoch++; clearObservation(); route = ManagementRoute.Decision(row.intent)
    }
    fun choose(recipient: String) {
        val selected = route as? ManagementRoute.Transfer ?: return
        if (frozen(selected.operation) || peers.none { it.id == recipient }) return
        route = selected.copy(recipient = recipient); clearObservation()
    }
    fun submit() {
        val selected = route as? ManagementRoute.Transfer ?: return
        val row = row(selected.operation)
        if (row != null) retry(row)
        else selected.recipient?.let { execute(ManagementRequest.Propose(selected.database, it, selected.operation)) }
    }
    fun decide(accept: Boolean) {
        val selected = route as? ManagementRoute.Decision ?: return
        if (row(selected.intent.operation)?.phase != ManagementPhase.WAITING_DECISION || decisionPending(selected.intent.operation)) return
        execute(ManagementRequest.Decision(selected.intent, accept))
    }
    fun retry(row: ManagementHandoffView) {
        if (row.intent.database != database || terminal(row.phase)) return
        val original = retained[Key(row.intent.database, row.intent.operation)]
        // A failed first explicit decision must still be made; later phases resume
        // the stored decision and publication without signing or proposing again.
        execute(if (row.phase == ManagementPhase.WAITING_DECISION && original is ManagementRequest.Decision)
            original else ManagementRequest.Retry(row.intent))
    }
    fun retryUnconfirmed(operation: String) {
        val selected = database ?: return
        retained[Key(selected, operation)]?.let(::execute)
    }
    fun row(operation: String) = rows.firstOrNull { it.intent.operation == operation }
    fun busy(operation: String): Boolean = database?.let { working[Key(it, operation)] == true } == true
    fun problem(operation: String): ManagementProblem? = database?.let { problems[Key(it, operation)] }
    fun frozen(operation: String): Boolean = database?.let { retained.containsKey(Key(it, operation)) } == true || row(operation) != null
    fun decisionPending(operation: String): Boolean = busy(operation) || database?.let { retained.containsKey(Key(it, operation)) } == true ||
        row(operation)?.phase != ManagementPhase.WAITING_DECISION
    fun unconfirmedProposals(): List<Pair<String, String>> = retained.values.filterIsInstance<ManagementRequest.Propose>()
        .filter { it.database == database && row(it.operation) == null }.map { it.operation to it.recipient }

    /** No network runs on composition, input or protected-row refresh. This is an explicit RPC. */
    fun inspectPeer(row: ManagementHandoffView) {
        if (row.intent.database != database || observing) return
        val expected = epoch; val display = routeEpoch
        clearObservation(); observing = true
        scope.launch {
            try {
                val owner = host()
                val value = withContext(Dispatchers.IO) { owner.managementStatus(row.intent) }
                if (epoch == expected && routeEpoch == display && database == row.intent.database && value.intent == row.intent)
                    observation = value
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == expected && routeEpoch == display) observationFailed = true }
            finally { observing = false }
        }
    }
    fun close() { routeEpoch++; route = null; preparing = false; clearObservation() }
    fun lock() { epoch++; loading = false; close(); load() }
    private fun clearObservation() { observation = null; observationFailed = false }

    private fun execute(request: ManagementRequest) {
        val key = Key(request.database, request.operation)
        if (database != request.database || working[key] == true) return
        val binding = session()?.takeIf { it.database == request.database && current(it) }
        if (request !is ManagementRequest.Retry && binding == null) { problems[key] = ManagementProblem.NEEDS_UNLOCK; return }
        val expected = epoch; val display = routeEpoch
        retained[key] = request; working[key] = true; problems.remove(key); clearObservation()
        scope.launch {
            try {
                val owner = host()
                val result = withContext(Dispatchers.IO) {
                    when (request) {
                        is ManagementRequest.Propose -> binding!!.session.proposeManagement(request.recipient, request.operation)
                        is ManagementRequest.Decision -> if (request.accept) binding!!.session.acceptManagement(request.intent) else binding!!.session.rejectManagement(request.intent)
                        is ManagementRequest.Retry -> owner.retryManagement(request.intent)
                    }
                }
                if (epoch == expected && database == request.database && (binding == null || current(binding))) {
                    // A profile read started before this ACK cannot restore an earlier decision phase.
                    projectionEpoch++
                    rows = rows.filterNot { it.intent.operation == request.operation } + result
                    retained.remove(key); problems.remove(key)
                    binding?.let(changed)
                    if (request is ManagementRequest.Decision && routeEpoch == display) close()
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) {
                if (database == request.database) problems[key] = problem(error)
            } finally {
                working.remove(key)
                if (database == request.database) load()
            }
        }
    }

    private fun applyProjection(saved: List<ManagementHandoffView>, devices: List<DeviceRow>, sequence: Long) {
        appliedProjection = sequence
        rows = saved; peers = devices.filter { !it.local }; localDevice = devices.singleOrNull { it.local }?.id
        loaded = true; loadFailed = false
        saved.filter { terminal(it.phase) }.forEach { row ->
            val key = Key(row.intent.database, row.intent.operation)
            retained.remove(key); problems.remove(key)
        }
    }
}

internal fun terminal(phase: ManagementPhase) = phase == ManagementPhase.REJECTED || phase == ManagementPhase.COMPLETED
private fun problem(error: Exception): ManagementProblem = if (error is AndroidException.Management) when (error.failure) {
    ManagementFailure.NEEDS_UNLOCK -> ManagementProblem.NEEDS_UNLOCK
    ManagementFailure.PENDING_APPLY -> ManagementProblem.PENDING_APPLY
    ManagementFailure.STALE -> ManagementProblem.STALE
    ManagementFailure.WRONG_ROLE -> ManagementProblem.WRONG_ROLE
    ManagementFailure.UNKNOWN -> ManagementProblem.UNKNOWN
    ManagementFailure.CONFLICT -> ManagementProblem.CONFLICT
    ManagementFailure.CAPACITY -> ManagementProblem.CAPACITY
    ManagementFailure.UNSUPPORTED -> ManagementProblem.UNSUPPORTED
} else ManagementProblem.UNCONFIRMED
private fun newManagementOperation(): String = ByteArray(32).also { SecureRandom().nextBytes(it) }
    .joinToString("") { (it.toInt() and 255).toString(16).padStart(2, '0') }
