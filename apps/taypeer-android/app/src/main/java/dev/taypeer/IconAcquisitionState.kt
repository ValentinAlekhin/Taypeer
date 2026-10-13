package dev.taypeer

import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.*

/** Immutable acquisition intent. Rust owns validation, original bytes and stage receipts. */
internal sealed interface IconSource {
    data class File(val uri: Uri) : IconSource
    data class Download(val value: IconDownload) : IconSource
    data class Stored(val identity: String) : IconSource
}

internal data class IconTarget(val database: String, val draft: String)
internal data class IconRequest(val target: IconTarget, val source: IconSource, val operation: String)
internal data class IconAttempt(val request: IconRequest, val busy: Boolean, val token: Long, val prepared: Boolean = false, val failure: IconFailure? = null, val uncertain: Boolean = false)
internal data class IconFileSelection(val target: IconTarget, val operation: String, val uri: Uri? = null)

/** A late reply can settle its own immutable intent, but never apply another form's projection. */
internal class IconAcquisitionState {
    private var sequence = 0L
    private val attempts = mutableStateMapOf<IconTarget, IconAttempt>()
    private val failures = mutableStateMapOf<IconTarget, IconFailure>()
    private val rejections = mutableStateMapOf<IconTarget, Boolean>()
    private val confirmations = mutableStateMapOf<IconTarget, Long>()
    var file by mutableStateOf<IconFileSelection?>(null); private set

    fun attempt(target: IconTarget?): IconAttempt? = target?.let(attempts::get)
    fun failure(target: IconTarget?): IconFailure? = target?.let(failures::get)
    fun rejected(target: IconTarget?): Boolean = target?.let(rejections::get) == true
    fun confirmation(target: IconTarget?): Long = target?.let(confirmations::get) ?: 0L
    fun blocked(target: IconTarget?): Boolean = attempt(target) != null
    fun selectFile(value: IconFileSelection?) { file = value }
    fun begin(request: IconRequest): Long? {
        val old = attempts[request.target]
        if (old != null && (old.busy || old.request != request)) return null
        failures.remove(request.target); rejections.remove(request.target)
        val token = ++sequence
        attempts[request.target] = IconAttempt(request, busy = true, token = token, prepared = old?.prepared == true)
        return token
    }
    fun prepared(request: IconRequest, token: Long) {
        val attempt = attempts[request.target] ?: return
        if (attempt.token == token) attempts[request.target] = attempt.copy(prepared = true)
    }
    fun confirmed(request: IconRequest, token: Long): Boolean {
        if (attempts[request.target]?.token != token) return false
        attempts.remove(request.target); failures.remove(request.target); rejections.remove(request.target)
        confirmations[request.target] = confirmation(request.target) + 1
        if (file?.target == request.target && file?.operation == request.operation) file = null
        return true
    }
    fun failed(request: IconRequest, token: Long, failure: Exception): Boolean {
        if (attempts[request.target]?.token != token) return false
        if (failure is AndroidException.Icon) {
            attempts.remove(request.target); failures[request.target] = failure.failure
        } else if (failure is AndroidException.InvalidInput || failure is AndroidException.InvalidOptions) {
            attempts.remove(request.target); rejections[request.target] = true
        } else {
            attempts[request.target] = IconAttempt(request, busy = false, token = token, prepared = attempts[request.target]?.prepared == true, uncertain = true)
        }
        return true
    }
    fun invalidateBinding() {
        // A revoked generation is not evidence that a staged operation had no effect.
        attempts.keys.toList().forEach { target -> attempts[target]?.let { attempts[target] = it.copy(busy = false, uncertain = true) } }
        failures.clear(); rejections.clear(); confirmations.clear()
    }
    fun discardFile(): IconFileSelection? = file.also { file = null }
}
