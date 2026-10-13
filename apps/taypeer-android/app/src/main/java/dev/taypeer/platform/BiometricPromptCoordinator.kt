package dev.taypeer.platform

import android.os.Looper
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import java.lang.ref.WeakReference

/** Application-owned private prompt lifecycle. All hooks run on the main thread. */
internal class BiometricPromptCoordinator(
    private val masterPasswordLabel: () -> String,
    private val onError: (BiometricPromptRequest, Int) -> Unit = { _, _ -> },
) {
    private class Active(request: BiometricPromptRequest) {
        var request: BiometricPromptRequest? = request
        var cryptoObject: BiometricPrompt.CryptoObject? = request.cryptoObject
        var prompt: BiometricPrompt? = null
        var retired = false
        var authenticated = false
        var terminal = false
    }

    private var activity: WeakReference<FragmentActivity>? = null
    private var active: Active? = null

    val isActive: Boolean get() { requireMainThread(); return active != null }
    val awaitingDismissal: Boolean get() { requireMainThread(); return active?.retired == true }

    /** Call in every Activity.onCreate. AndroidX rebinds the callback without reauthentication. */
    fun attach(activity: FragmentActivity) {
        requireMainThread()
        this.activity = WeakReference(activity)
        active?.let {
            it.prompt = prompt(activity, it)
            // Lock may have retired the request between Activities. Rebind its
            // terminal callback and repeat OS cancellation, never authentication.
            if (it.retired) it.prompt?.cancelAuthentication()
        }
    }

    /** True means authenticate was requested, never that the document is OPEN. */
    fun show(request: BiometricPromptRequest): Boolean {
        requireMainThread()
        if (active != null) return false
        val activity = activity?.get() ?: return false
        if (activity.isFinishing || activity.isDestroyed ||
            !activity.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) return false

        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(request.title)
            .setSubtitle(request.subtitle)
            .setNegativeButtonText(masterPasswordLabel())
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .build()
        val selected = Active(request)
        val prompt = prompt(activity, selected)
        selected.prompt = prompt
        active = selected
        try {
            prompt.authenticate(info, checkNotNull(selected.cryptoObject))
        } catch (failure: RuntimeException) {
            // authenticate may have partially started an OS request. Keep the
            // dismissal barrier until its terminal callback, even on startup failure.
            try {
                retire(selected, BiometricCancellation.CANCEL)
                onError(request, BiometricPrompt.ERROR_UNABLE_TO_PROCESS)
            } finally {
                prompt.cancelAuthentication()
            }
            throw failure
        }
        return true
    }

    fun cancel(reason: BiometricCancellation) {
        requireMainThread()
        val selected = active ?: return
        try { retire(selected, reason) }
        finally { selected.prompt?.cancelAuthentication() }
    }

    /** A delayed private cleanup must never cancel a replacement request. */
    fun cancel(request: BiometricPromptRequest, reason: BiometricCancellation) {
        requireMainThread()
        if (active?.request !== request) return
        cancel(reason)
    }

    /** Recreation keeps the same request. A destroyed screen never fabricates OS dismissal. */
    fun detach(activity: FragmentActivity, changingConfigurations: Boolean) {
        requireMainThread()
        if (this.activity?.get() !== activity) return
        try {
            if (!changingConfigurations) cancel(BiometricCancellation.BACKGROUND)
        } finally {
            this.activity = null
            // Retained AndroidX state owns the OS request; attach installs a
            // callback for the same Active object in the replacement Activity.
            active?.prompt = null
        }
    }

    private fun prompt(activity: FragmentActivity, selected: Active): BiometricPrompt =
        BiometricPrompt(activity, ContextCompat.getMainExecutor(activity),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    requireMainThread()
                    if (!current(selected)) return
                    try {
                        if (!selected.retired && !selected.authenticated) {
                            val expected = selected.cryptoObject?.cipher
                            if (expected == null || result.cryptoObject?.cipher !== expected ||
                                result.authenticationType != BiometricPrompt.AUTHENTICATION_RESULT_TYPE_BIOMETRIC) {
                                retire(selected, BiometricCancellation.CANCEL)
                                selected.request?.let { onError(it, BiometricPrompt.ERROR_UNABLE_TO_PROCESS) }
                            } else {
                                selected.authenticated = true
                                try { selected.request?.authenticated(result) }
                                catch (failure: RuntimeException) {
                                    // A private adapter that failed to enqueue its
                                    // completion still owns a channel which must close.
                                    retire(selected, BiometricCancellation.CANCEL)
                                    throw failure
                                }
                            }
                        }
                    } finally { dismissed(selected) }
                }

                override fun onAuthenticationError(code: Int, message: CharSequence) {
                    requireMainThread()
                    if (!current(selected)) return
                    try {
                        retire(selected, if (code == BiometricPrompt.ERROR_NEGATIVE_BUTTON)
                            BiometricCancellation.MASTER_PASSWORD else BiometricCancellation.CANCEL)
                        selected.request?.let { onError(it, code) }
                    } finally { dismissed(selected) }
                }
            })

    private fun retire(selected: Active, reason: BiometricCancellation) {
        if (!current(selected) || selected.retired) return
        selected.retired = true
        selected.request?.retire(reason)
    }

    private fun dismissed(selected: Active) {
        if (!current(selected)) return
        selected.terminal = true
        active = null
        val request = selected.request
        selected.request = null
        selected.cryptoObject = null
        selected.prompt = null
        request?.promptDismissed()
    }

    private fun current(selected: Active): Boolean = active === selected && !selected.terminal

    private fun requireMainThread() {
        check(Looper.myLooper() === Looper.getMainLooper()) { "Biometric prompt hook requires main thread" }
    }
}
