package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.*
import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicBoolean

/** Local setup projection; Native alone grants setup and authenticated worker access. */
internal class BiometryUiState(private val scope: CoroutineScope,
    private val prepareEnrollment: (DocumentSession) -> BiometricEnrollment = { it.prepareBiometricEnrollment() },
    private val host: suspend () -> Host) {
    private data class Selection(val path: String, val database: String, val root: String)
    /** Each request owns its preparation; an old completion cannot replace a newer handle. */
    private class Attempt(operation: String? = null) {
        private val retired = AtomicBoolean()
        private val enrollment = AtomicReference<BiometricEnrollment?>()
        val operation = AtomicReference(operation)
        fun current() = !retired.get()
        fun install(prepared: BiometricEnrollment): Boolean {
            if (retired.get()) { prepared.cancel(); return false }
            enrollment.set(prepared)
            // Retirement can occur between the initial check and publication.
            if (retired.get()) {
                if (enrollment.compareAndSet(prepared, null)) prepared.cancel()
                return false
            }
            return true
        }
        fun release(prepared: BiometricEnrollment) { enrollment.compareAndSet(prepared, null) }
        fun retire() { retired.set(true); enrollment.getAndSet(null)?.cancel() }
    }
    private var selected: Selection? = null
    private var revision = 0L
    private val active = AtomicReference<Attempt?>()
    var retryOperation by mutableStateOf<String?>(null); private set
    var status by mutableStateOf<BiometryStatusView?>(null); private set
    var busy by mutableStateOf(false); private set
    var failed by mutableStateOf(false); private set
    var cancelled by mutableStateOf(false); private set

    fun load(copy: WorkingCopyView) {
        val selection = Selection(copy.path, copy.database, copy.root)
        if (selection != selected) { clear(); selected = selection }
        if (busy) return
        perform(copy) { owner, _ -> owner.biometryStatus(copy) }
    }
    fun enable(copy: WorkingCopyView, session: DocumentSession) {
        if (busy) return
        perform(copy) { _, attempt ->
            val prepared = prepareEnrollment(session)
            try {
                if (!attempt.install(prepared)) throw AndroidException.Biometry(BiometryFailure.STALE)
                attempt.operation.set(prepared.identity().attempt)
                if (!attempt.current()) throw AndroidException.Biometry(BiometryFailure.STALE)
                prepared.confirm()
            }
            finally { attempt.release(prepared); prepared.close() }
        }
    }
    fun disable(copy: WorkingCopyView) {
        val generation = status?.generation ?: return
        val operation = newOperation()
        perform(copy, operation) { owner, _ -> owner.disableBiometry(copy, generation, operation) }
    }
    fun retry(copy: WorkingCopyView) {
        val operation = status?.operation ?: retryOperation ?: return
        perform(copy, operation) { owner, _ -> owner.retryBiometricSetup(copy, operation) }
    }
    private fun perform(copy: WorkingCopyView, operation: String? = null, action: (Host, Attempt) -> BiometryStatusView) {
        if (busy) return
        val selection = Selection(copy.path, copy.database, copy.root)
        if (selection != selected) { clear(); selected = selection }
        val requested = ++revision
        val attempt = Attempt(operation)
        active.getAndSet(attempt)?.retire()
        busy = true; failed = false; cancelled = false
        scope.launch {
            try {
                val owner = host()
                val value = withContext(Dispatchers.IO) {
                    if (!attempt.current()) throw AndroidException.Biometry(BiometryFailure.STALE)
                    action(owner, attempt)
                }
                if (selected == selection && requested == revision) {
                    status = value; retryOperation = null
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (failure: Exception) {
                if (selected == selection && requested == revision) {
                    this@BiometryUiState.cancelled = failure is AndroidException.Biometry &&
                        failure.failure == BiometryFailure.CANCELLED
                    failed = !this@BiometryUiState.cancelled
                    if (failure is AndroidException.CommitUncertain) retryOperation = attempt.operation.get()
                    // A durable pending setup can outlive the failed OS/publication ACK.
                    val latest = try { withContext(Dispatchers.IO) { host().biometryStatus(copy) } }
                        catch (_: Exception) { null }
                    if (selected == selection && requested == revision && latest != null) status = latest
                }
            } finally {
                active.compareAndSet(attempt, null)
                attempt.retire()
                if (selected == selection && requested == revision) busy = false
            }
        }
    }
    fun clear() {
        active.getAndSet(null)?.retire() // Independent Native fence precedes OS/Binder cleanup.
        revision++; selected = null; status = null; busy = false; failed = false; cancelled = false
        retryOperation = null
    }
}
