package dev.taypeer

import androidx.compose.runtime.*
import dev.taypeer.bridge.*
import kotlinx.coroutines.*

internal data class GroupFaviconBinding(val session: DocumentSession, val database: String, val generation: ULong, val epoch: Long)
internal enum class GroupFaviconUiFailure { REJECTED, UNCONFIRMED }

/** Rust owns frozen selection, network policy, per-entry receipts and actual outcomes. */
internal class GroupFaviconsState(
    private val scope: CoroutineScope,
    private val binding: () -> GroupFaviconBinding?,
    private val current: (GroupFaviconBinding) -> Boolean,
    private val changed: (GroupFaviconBinding) -> Unit,
) {
    private data class Selection(val group: String, val recursive: Boolean, val replace: Boolean, val operation: String)
    private var epoch = 0L
    private var selected: GroupFaviconBinding? = null
    private var attempt: Selection? = null
    private var owner: GroupFaviconBatch? = null
    var group by mutableStateOf<GroupRow?>(null); private set
    var showing by mutableStateOf(false); private set
    var recursive by mutableStateOf(false); private set
    var replace by mutableStateOf(false); private set
    var busy by mutableStateOf(false); private set
    var rows by mutableStateOf<List<GroupFaviconRow>>(emptyList()); private set
    var failure by mutableStateOf<GroupFaviconUiFailure?>(null); private set
    val started get() = attempt != null
    val editable get() = !busy && !started
    val retryable get() = rows.any { it.state == GroupFaviconState.FAILED || it.state == GroupFaviconState.UNCERTAIN || it.state == GroupFaviconState.READY }

    /** Merely opening the FIG form performs no Native preparation or network acquisition. */
    fun open(value: GroupRow) {
        reset()
        val source = binding() ?: return
        if (!current(source)) return
        selected = source; group = value; showing = true
    }
    fun updateRecursive(value: Boolean) { if (editable) recursive = value }
    fun updateReplace(value: Boolean) { if (editable) replace = value }

    fun submit() {
        val source = selected ?: return
        val target = group ?: return
        if (busy || !showing || !current(source) || (owner != null && !retryable)) return
        val request = attempt ?: Selection(target.id, recursive, replace, newOperation()).also { attempt = it }
        val expected = epoch
        busy = true; failure = null
        scope.launch {
            var prepared = owner
            var progress: Job? = null
            try {
                if (prepared == null) {
                    withContext(Dispatchers.IO) {
                        // Own the return before the cancellable dispatcher handoff.
                        prepared = source.session.prepareGroupFavicons(request.group, request.recursive, request.replace, request.operation)
                    }
                    val created = prepared ?: throw AndroidException.Runtime()
                    if (!accepts(expected, source)) return@launch
                    owner = created
                    rows = withContext(Dispatchers.IO) { created.rows() }
                }
                val active = prepared ?: throw AndroidException.Runtime()
                progress = launch {
                    while (isActive) {
                        delay(100)
                        if (!accepts(expected, source)) break
                        val value = try { withContext(Dispatchers.IO) { active.rows() } }
                        catch (cancelled: CancellationException) { throw cancelled }
                        catch (error: Exception) { if (accepts(expected, source)) throw error else break }
                        if (!accepts(expected, source)) break
                        rows = value
                    }
                }
                val result = withContext(Dispatchers.IO) { active.run() }
                if (accepts(expected, source)) {
                    rows = result
                    // Shared per-entry ACK is the only reason to refresh product projection.
                    if (result.any { it.state == GroupFaviconState.APPLIED }) changed(source)
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) {
                if (accepts(expected, source)) {
                    failure = if (error is AndroidException.InvalidInput || error is AndroidException.InvalidOptions) GroupFaviconUiFailure.REJECTED else GroupFaviconUiFailure.UNCONFIRMED
                    // A typed preparation rejection permits correcting input; unknown
                    // publication keeps the original operation/options for exact retry.
                    if (owner == null && failure == GroupFaviconUiFailure.REJECTED) attempt = null
                }
            } finally {
                withContext(NonCancellable) {
                    progress?.cancelAndJoin()
                    val released = prepared
                    if (released != null && owner !== released) withContext(Dispatchers.IO) {
                        released.cancel(); released.close()
                    }
                }
                if (epoch == expected) busy = false
            }
        }
    }
    private fun accepts(expected: Long, source: GroupFaviconBinding) = epoch == expected && showing && selected == source && current(source)

    /** Called on lock, source change and ViewModel cleanup, independently of pending IO. */
    fun reset() {
        epoch++
        val previous = owner; owner = null
        val wasBusy = busy
        previous?.cancel()
        // The running coroutine owns its Native object until run/poll finally.
        // Dropping an idle batch only releases bounded in-memory rows; it performs
        // no platform/network IO. It must still happen after ViewModel scope cancel.
        if (previous != null && !wasBusy) previous.close()
        selected = null; attempt = null; group = null; showing = false; busy = false
        rows = emptyList(); failure = null; recursive = false; replace = false
    }
    fun close() = reset()
}
