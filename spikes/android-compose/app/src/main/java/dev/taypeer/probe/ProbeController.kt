package dev.taypeer.probe

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.input.TextFieldValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.taypeer.probe.bridge.ProbeSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Synthetic FFI probe. UI-owned text is deliberately never saved in a Bundle. */
class ProbeController : ViewModel() {
    private val session = ProbeSession()
    var input by mutableStateOf(TextFieldValue())
    var revealed by mutableStateOf("")
        private set
    var status by mutableStateOf(session.status())
        private set
    var message by mutableStateOf<Int?>(null)
    var dialog by mutableStateOf(false)
    private var uiGeneration = 0L
    internal var delayReply: suspend () -> Unit = { delay(3000) }
    private var foreground = false

    fun resume() { foreground = true }
    fun pause() { foreground = false; lockNow() }
    fun edit(value: TextFieldValue, generation: ULong) {
        // An old InputConnection may deliver a change after lock/background.
        if (foreground && generation == status.generation) input = value
    }

    fun lockNow() {
        // Invalidate UI replies before calling Rust, including already returned copies.
        uiGeneration++
        input = TextFieldValue()
        revealed = ""
        dialog = false
        message = null
        status = session.lock()
    }

    fun openSample() {
        val text = input.text
        if (text.codePointCount(0, text.length) !in 1..256) {
            message = R.string.invalid_input
            return
        }
        // This synthetic operation is bounded and synchronous. Production KDF is not.
        uiGeneration++
        try {
            status = session.openSample(text)
            input = TextFieldValue()
            revealed = ""
            message = null
        } catch (_: Exception) {
            message = R.string.error
        }
    }

    fun hide() { revealed = "" }

    fun reveal(delayed: Boolean = false) {
        val requestUiGeneration = uiGeneration
        val requestRustGeneration = status.generation
        message = if (delayed) R.string.waiting else null
        viewModelScope.launch {
            try {
                // Read first, then delay delivery: exercises even a reply issued before lock.
                val reply = withContext(Dispatchers.Default) {
                    session.reveal(requestRustGeneration)
                }
                if (delayed) delayReply()
                if (requestUiGeneration == uiGeneration && !status.locked &&
                    reply.generation == status.generation) {
                    revealed = reply.text
                    message = null
                } else {
                    message = R.string.discarded
                }
            } catch (_: Exception) {
                if (requestUiGeneration == uiGeneration) message = R.string.error
                else message = R.string.discarded
            }
        }
    }

    fun sampleForCopy(): String? = try {
        if (status.locked) null else session.reveal(status.generation).text
    } catch (_: Exception) { null }

    override fun onCleared() {
        lockNow()
        session.destroy()
    }
}
