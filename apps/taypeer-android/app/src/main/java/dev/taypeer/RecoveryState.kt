package dev.taypeer

import androidx.compose.runtime.*
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import kotlinx.coroutines.*

internal enum class RecoveryFailure { INPUT, UNCONFIRMED, NEW_OPERATION, SOURCE_PASSWORD }
internal data class RecoveryTarget(val source: WorkingCopyView, val operation: String)

/** UI owns unfinished input; Rust owns frozen sources, password attempts, proof and publication. */
internal class RecoveryState(
    private val scope: CoroutineScope,
    private val manager: suspend () -> DocumentManager,
    private val detached: () -> Unit,
    private val adopted: (WorkingCopyView, String) -> Unit,
) {
    private data class Attempt(val name: String, val password: String)
    private var epoch = 0L
    private var worker: DocumentSession? = null
    private var ownsReader = false
    private var attempt: Attempt? = null
    private var publication: ByteArray? = null
    var target by mutableStateOf<RecoveryTarget?>(null); private set
    var showing by mutableStateOf(false); private set
    var busy by mutableStateOf(false); private set
    var frozen by mutableStateOf(false); private set
    var failure by mutableStateOf<RecoveryFailure?>(null); private set
    var name by mutableStateOf(""); private set
    var password by mutableStateOf(""); private set
    var repeated by mutableStateOf(""); private set
    var pending by mutableStateOf<List<PendingRecoveryView>>(emptyList()); private set
    var pendingFailure by mutableStateOf(false); private set
    var pendingLoaded by mutableStateOf(false); private set
    val editable get() = !busy && !frozen

    fun updateName(value: String) { if (editable) { name = value; failure = null } }
    fun updatePassword(value: String) { if (editable) { password = value; failure = null } }
    fun updateRepeated(value: String) { if (editable) { repeated = value; failure = null } }

    /** Called after the existing Save / Discard / Stay decision, with no automatic form save. */
    fun begin(source: WorkingCopyView, session: DocumentSession?, operation: String = newOperation()) {
        if (busy || frozen) return
        epoch++
        target = RecoveryTarget(source, operation)
        worker = session
        ownsReader = false
        showing = true; failure = null
        name = ""; password = ""; repeated = ""; attempt = null; publication = null
    }

    /** Explicit constrained authentication is needed for a fork or a retained source after lock. */
    fun authenticate(source: WorkingCopyView, sourcePassword: String, operation: String) {
        if (busy) return
        val expected = ++epoch
        worker = null; ownsReader = false
        target = RecoveryTarget(source, operation); showing = true; busy = true; failure = null
        name = ""; password = ""; repeated = ""; attempt = null; publication = null; frozen = false
        detached()
        scope.launch {
            var opened: DocumentSession? = null
            try {
                val owner = manager()
                withContext(Dispatchers.IO) { owner.closeRecoverySource(source.database); opened = owner.openRecovery(source, sourcePassword, operation) }
                if (epoch != expected) {
                    opened?.let { withContext(Dispatchers.IO) { owner.close(it) } }
                    return@launch
                }
                worker = opened; ownsReader = true; busy = false
            } catch (cancelled: CancellationException) {
                opened?.let { value -> withContext(NonCancellable + Dispatchers.IO) { manager().close(value) } }
                throw cancelled
            } catch (error: Exception) {
                if (epoch == expected) {
                    busy = false
                    failure = if (error is AndroidException.RecoveryRequiresNewOperation) RecoveryFailure.NEW_OPERATION else RecoveryFailure.SOURCE_PASSWORD
                    frozen = failure == RecoveryFailure.NEW_OPERATION
                }
            }
        }
    }

    fun submit() {
        val selected = target ?: return
        if (busy || password != repeated) return
        val request = attempt ?: Attempt(name, password).also { attempt = it }
        val expected = epoch
        busy = true; frozen = true; failure = null
        scope.launch {
            try {
                val owner = manager()
                var receipt = publication
                if (receipt == null) {
                    val active = worker ?: throw AndroidException.Runtime()
                    receipt = withContext(Dispatchers.IO) { active.recoverWorkingCopy(selected.operation, request.name, request.password) }
                    if (epoch != expected) return@launch
                    publication = receipt
                    // Detach the UI before intentionally closing its old source process.
                    detached()
                }
                if (epoch != expected) return@launch
                val result = withContext(Dispatchers.IO) { owner.adoptRecovery(receipt, selected.source.database) }
                if (epoch != expected) return@launch
                worker = null; ownsReader = false; publication = null; attempt = null; frozen = false; busy = false; showing = false
                val confirmedPassword = request.password
                name = ""; password = ""; repeated = ""; target = null; failure = null
                reload()
                adopted(result, confirmedPassword)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) {
                if (epoch != expected) return@launch
                busy = false
                when (error) {
                    is AndroidException.InvalidInput, is AndroidException.InvalidOptions -> {
                        // Only these typed precommit field rejections allow correction.
                        if (publication == null) { attempt = null; frozen = false; failure = RecoveryFailure.INPUT }
                        else failure = RecoveryFailure.UNCONFIRMED
                    }
                    is AndroidException.RecoveryRequiresNewOperation -> failure = RecoveryFailure.NEW_OPERATION
                    else -> failure = RecoveryFailure.UNCONFIRMED
                }
            }
        }
    }

    /** A proved absent candidate permits a separate explicit action, never a silent new ID. */
    fun newOperationRequested(): RecoveryTarget? {
        val selected = target ?: return null
        if (busy || failure != RecoveryFailure.NEW_OPERATION) return null
        worker = null; ownsReader = false; attempt = null; publication = null; frozen = false; showing = false
        name = ""; password = ""; repeated = ""; failure = null; target = null; epoch++
        return RecoveryTarget(selected.source, newOperation())
    }

    /** This phase comes from protected Native reconciliation; the next action still gets a new ID. */
    fun showRefusal(row: PendingRecoveryView) {
        if (busy || row.phase != RecoveryPhase.REFUSED_BEFORE_PUBLICATION) return
        epoch++; target = RecoveryTarget(row.source, row.operation); worker = null; ownsReader = false
        showing = true; busy = false; frozen = true; failure = RecoveryFailure.NEW_OPERATION
        name = ""; password = ""; repeated = ""; attempt = null; publication = null
    }

    fun close() {
        if (busy || frozen) return
        val active = worker?.takeIf { ownsReader }
        worker = null; ownsReader = false; showing = false; target = null; epoch++
        val expected = epoch
        name = ""; password = ""; repeated = ""; attempt = null; publication = null; failure = null
        if (active != null) scope.launch {
            try { withContext(Dispatchers.IO) { manager().close(active) }; if (epoch == expected) reload() }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == expected) pendingFailure = true }
        }
    }

    /** Background revokes plaintext input and replies, retaining only protected restart discovery. */
    fun lock() {
        val owned = worker?.takeIf { ownsReader }
        epoch++; worker = null; ownsReader = false; showing = false; busy = false; frozen = false
        val expected = epoch
        name = ""; password = ""; repeated = ""; attempt = null; publication = null; target = null; failure = null
        if (owned != null) scope.launch {
            try { withContext(Dispatchers.IO) { manager().close(owned) } }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == expected) pendingFailure = true }
        }
        reload()
    }
    fun reload() {
        val expected = epoch
        scope.launch {
            try {
                val rows = withContext(Dispatchers.IO) { manager().pendingRecoveries() }
                if (epoch == expected) { pending = rows; pendingFailure = false; pendingLoaded = true }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (epoch == expected) { pendingFailure = true; pendingLoaded = true } }
        }
    }
}
