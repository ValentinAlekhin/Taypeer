package dev.taypeer

import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.EncryptedDatabaseCopy
import dev.taypeer.bridge.EncryptedDatabaseCopyIdentity
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.SelectedOutput
import dev.taypeer.bridge.WorkingCopyView
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal enum class CiphertextCopyPhase { CAPTURING, READY, SELECTING, WRITING, SAVED, FAILED }
internal enum class CiphertextCopyFailure { CAPTURE, OUTPUT, UNCONFIRMED }
internal data class CiphertextCopyPicker(val request: String, val filename: String)

/** The host owns ciphertext. This owner holds only its opaque capture and public UI intent.
 * Normal worker/background revocation does not cancel a selected encrypted copy.
 * Explicit route/copy/profile changes revoke the attempt; an old callback cannot acknowledge it.
 */
internal class CiphertextCopyState(
    private val scope: CoroutineScope,
    private val host: suspend () -> Host,
    private val currentSource: () -> WorkingCopyView?,
    private val output: (Uri) -> SelectedOutput,
) {
    private class Attempt(val source: WorkingCopyView, val filename: String) {
        val operation = newOperation()
        var capture: EncryptedDatabaseCopy? = null
        var picker: String? = null
    }
    private var attempt: Attempt? = null
    // ViewModel cancellation must still release a capture or a late native return.
    private val cleanup = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    var showing by mutableStateOf(false); private set
    var filename by mutableStateOf(""); private set
    var phase by mutableStateOf(CiphertextCopyPhase.CAPTURING); private set
    var failure by mutableStateOf<CiphertextCopyFailure?>(null); private set
    var identity by mutableStateOf<EncryptedDatabaseCopyIdentity?>(null); private set
    val operation get() = attempt?.operation
    val pendingPicker get() = attempt?.picker?.let { CiphertextCopyPicker(it, filename) }
    val canChoose get() = showing && attempt?.capture != null && phase in setOf(
        CiphertextCopyPhase.READY, CiphertextCopyPhase.FAILED, CiphertextCopyPhase.SAVED,
    )

    fun begin(source: WorkingCopyView, filename: String) {
        cancel()
        val selected = Attempt(source, filename)
        attempt = selected
        this.filename = filename
        showing = true; phase = CiphertextCopyPhase.CAPTURING; failure = null; identity = null
        scope.launch { capture(selected) }
    }

    private suspend fun capture(selected: Attempt) {
        var captured: EncryptedDatabaseCopy? = null
        try {
            val owner = host()
            withContext(Dispatchers.IO) { captured = owner.captureDatabaseCopy(selected.source) }
            val value = captured ?: throw AndroidException.Runtime()
            if (!current(selected)) { release(value); discardIfSelected(selected); return }
            val capturedIdentity = value.identity()
            selected.capture = value
            identity = capturedIdentity
            phase = CiphertextCopyPhase.READY
        } catch (cancelled: CancellationException) {
            captured?.let(::release)
            throw cancelled
        } catch (_: Exception) {
            captured?.let(::release)
            if (current(selected)) { phase = CiphertextCopyPhase.FAILED; failure = CiphertextCopyFailure.CAPTURE }
            else discardIfSelected(selected)
        }
    }

    /** Retries retain this exact capture and ask explicitly for a new destination. */
    fun preparePicker(): CiphertextCopyPicker? {
        val selected = attempt ?: return null
        if (!current(selected)) { cancel(); return null }
        if (!canChoose) return null
        val request = newOperation()
        selected.picker = request
        phase = CiphertextCopyPhase.SELECTING; failure = null
        return CiphertextCopyPicker(request, selected.filename)
    }

    fun pickerFailed(request: String) {
        val selected = attempt ?: return
        if (selected.picker != request) return
        selected.picker = null
        if (current(selected)) { phase = CiphertextCopyPhase.FAILED; failure = CiphertextCopyFailure.OUTPUT }
        else cancel()
    }

    /** The URI is used by this one I/O call and never retained as a future Save destination. */
    fun selected(request: String?, uri: Uri?) {
        val selected = attempt ?: return
        if (request == null || selected.picker != request || phase != CiphertextCopyPhase.SELECTING) return
        selected.picker = null
        if (!current(selected) || uri == null) { cancel(); return }
        val captured = selected.capture ?: return
        phase = CiphertextCopyPhase.WRITING; failure = null
        scope.launch {
            try {
                withContext(Dispatchers.IO) {
                    val destination = output(uri)
                    try { captured.export(destination) }
                    finally { destination.close() }
                }
                if (current(selected)) phase = CiphertextCopyPhase.SAVED
                else discardIfSelected(selected)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                if (current(selected)) {
                    if (error is AndroidException.ExportCancelled) cancel()
                    else {
                        phase = CiphertextCopyPhase.FAILED
                        failure = if (error is AndroidException.CommitUncertain) CiphertextCopyFailure.UNCONFIRMED else CiphertextCopyFailure.OUTPUT
                    }
                } else discardIfSelected(selected)
            }
        }
    }

    /** A failed capture has no destination/ciphertext lease to silently retry. */
    fun retryCapture() {
        val selected = attempt ?: return
        if (phase != CiphertextCopyPhase.FAILED || failure != CiphertextCopyFailure.CAPTURE) return
        if (!current(selected)) { cancel(); return }
        begin(selected.source, selected.filename)
    }

    fun cancel() {
        val old = attempt
        attempt = null; showing = false; identity = null; filename = ""; failure = null
        old?.capture?.let(::release)
        old?.capture = null; old?.picker = null
    }
    private fun current(selected: Attempt): Boolean {
        val source = currentSource()
        return attempt === selected && source != null && source.database == selected.source.database &&
            source.root == selected.source.root && source.path == selected.source.path
    }
    private fun discardIfSelected(selected: Attempt) { if (attempt === selected) cancel() }
    private fun release(captured: EncryptedDatabaseCopy) {
        cleanup.launch {
            try { captured.cancel() }
            finally { captured.close() }
        }
    }
}
