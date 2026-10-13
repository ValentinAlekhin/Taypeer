package dev.taypeer.platform

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.fragment.app.FragmentActivity
import dev.taypeer.R
import dev.taypeer.TaypeerApplication
import dev.taypeer.bridge.*
import java.security.SecureRandom
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/** Fixed application bootstrap port. Neither Compose nor a file can replace its OS keys. */
internal class AndroidOsBiometry(context: Context) : OsBiometryPort {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val crypto = Executors.newCachedThreadPool { task -> Thread(task, "biometric-os-crypto") }
    private val leases = ConcurrentHashMap<String, Lease>()
    private val active = AtomicReference<Request?>()
    /** Test barrier after actual OS success; never supplies auth, key bytes or a native reply. */
    @Volatile internal var beforeProvider: (() -> Unit)? = null
    private val coordinator = BiometricPromptCoordinator({ text(R.string.biometric_master_password) })

    private class Lease(val key: BiometricOsKey, val binding: BiometricBindingView, val attempt: String) {
        val used = AtomicBoolean()
        val retired = AtomicBoolean()
        val request = AtomicReference<Request?>()
    }

    fun attach(activity: FragmentActivity) { coordinator.attach(activity) }
    fun detach(activity: FragmentActivity, changingConfigurations: Boolean) {
        coordinator.detach(activity, changingConfigurations)
    }
    fun background() {
        // The request's private writer and Native fence retire before AndroidX cancellation.
        active.get()?.retire(BiometricCancellation.BACKGROUND)
        coordinator.cancel(BiometricCancellation.BACKGROUND)
    }

    override fun availability(): BiometricAvailability = when (
        BiometricManager.from(context).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
    ) {
        BiometricManager.BIOMETRIC_SUCCESS -> BiometricAvailability.AVAILABLE
        BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE -> BiometricAvailability.NO_HARDWARE
        BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> BiometricAvailability.NO_ENROLLMENT
        else -> BiometricAvailability.UNAVAILABLE
    }

    override fun createKey(binding: BiometricBindingView, attempt: String): OsBiometricKeyLease = guarded {
        validate(binding, attempt)
        if (availability() != BiometricAvailability.AVAILABLE) unavailable()
        val key = BiometricOsKey.create(context, binding.scopeDigest)
        try { mint(key, binding, attempt) }
        catch (failure: Exception) {
            try { key.delete() } catch (_: Exception) { /* No Native wrapper has been published. */ }
            throw failure
        }
    }

    override fun resolveKey(binding: BiometricBindingView, generation: String, alias: String,
        spkiDigest: String, attempt: String): OsBiometricKeyLease = guarded {
        validate(binding, attempt)
        require(generation.matches(Regex("[0-9a-f]{32}")))
        require(alias == "taypeer_biometry_v1_${binding.scopeDigest}_$generation")
        mint(BiometricOsKey.registeredDigest(alias, binding.scopeDigest, spkiDigest), binding, attempt)
    }

    private fun mint(key: BiometricOsKey, binding: BiometricBindingView, attempt: String): OsBiometricKeyLease {
        val id = ByteArray(32).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it.toInt() and 255) }
        val alias = key.registryAlias()
        val public = key.publicSpki()
        if (public.isEmpty() || public.size > 4096) unavailable()
        val owner = Lease(key, binding, attempt)
        if (leases.putIfAbsent(id, owner) != null) unavailable()
        return OsBiometricKeyLease(id = id, generation = alias.takeLast(32), binding = binding,
            attempt = attempt, alias = alias, spki = public)
    }

    override fun runPrompt(call: BiometricPromptCall): BiometricPromptOutcome {
        if (Looper.myLooper() === Looper.getMainLooper()) unavailable()
        val id = call.leaseId()
        val owner = leases[id] ?: return BiometricPromptOutcome.UNAVAILABLE
        if (owner.retired.get() || !owner.used.compareAndSet(false, true)) return BiometricPromptOutcome.CANCELLED
        var request: Request? = null
        var input: ParcelFileDescriptor? = null
        try {
            val spec = call.scope()
            if (!sameBinding(owner.binding, spec) || owner.attempt != spec.attempt) return BiometricPromptOutcome.FAILED
            val cipher = owner.key.decryptCipher()
            val wrapped = call.wrappedCiphertext()
            if (wrapped.size != 256) { wrapped.fill(0); return BiometricPromptOutcome.FAILED }
            val pipe = ParcelFileDescriptor.createPipe()
            input = pipe[0]
            val writer = try { BiometricKeyPipe.open(pipe[1]) }
            catch (failure: Exception) { pipe[0].close(); wrapped.fill(0); throw failure }
            val selected = Request(call, cipher, wrapped, writer)
            request = selected
            owner.request.set(selected)
            if (owner.retired.get()) { selected.retire(BiometricCancellation.CANCEL); return BiometricPromptOutcome.CANCELLED }
            // Native duplicates the held read end synchronously inside the isolated UID.
            pipe[0].use { call.installInput(it.fd) }
            if (owner.retired.get()) { selected.retire(BiometricCancellation.CANCEL); return BiometricPromptOutcome.CANCELLED }
            if (!active.compareAndSet(null, selected)) { selected.retire(BiometricCancellation.CANCEL); return BiometricPromptOutcome.UNAVAILABLE }
            main.post {
                if (selected.retired.get()) return@post
                try {
                    if (!coordinator.show(selected)) selected.retire(BiometricCancellation.CANCEL)
                } catch (_: RuntimeException) { selected.retire(BiometricCancellation.CANCEL) }
            }
            return selected.result.get(120, TimeUnit.SECONDS)
        } catch (_: Exception) {
            request?.retire(BiometricCancellation.CANCEL)
            return if (owner.retired.get()) BiometricPromptOutcome.CANCELLED else BiometricPromptOutcome.FAILED
        } finally {
            try { input?.close() } catch (_: Exception) { /* Native owns its accepted duplicate. */ }
            request?.let { selected ->
                selected.dispose()
                active.compareAndSet(selected, null)
            }
            if (request == null) call.retireInput()
            leases.remove(id, owner)
        }
    }

    override fun retireLease(id: String) {
        val owner = leases.remove(id) ?: return
        owner.retired.set(true)
        owner.request.get()?.let { request ->
            request.retire(BiometricCancellation.CANCEL)
            main.post { coordinator.cancel(request, BiometricCancellation.CANCEL) }
        }
    }

    override fun deleteKey(binding: BiometricBindingView, generation: String, alias: String) = guarded {
        validate(binding, "0".repeat(64))
        leases.entries.filter { it.value.binding == binding && it.value.key.registryAlias() == alias }
            .forEach { retireLease(it.key) }
        BiometricOsKey.deleteRegistered(alias, binding.scopeDigest, generation)
    }

    private inner class Request(private val call: BiometricPromptCall, cipher: javax.crypto.Cipher,
        private val wrapped: ByteArray, private val writer: BiometricKeyPipe) : BiometricPromptRequest {
        val retired = AtomicBoolean()
        val result = CompletableFuture<BiometricPromptOutcome>()
        private val queued = AtomicBoolean()
        override val title get() = text(R.string.biometric_prompt_title)
        override val subtitle get() = text(R.string.biometric_prompt_subtitle)
        override val cryptoObject = BiometricPrompt.CryptoObject(cipher)

        override fun authenticated(result: BiometricPrompt.AuthenticationResult) {
            if (retired.get() || !queued.compareAndSet(false, true)) return
            val cipher = result.cryptoObject?.cipher
            if (cipher == null || cipher !== cryptoObject.cipher ||
                result.authenticationType != BiometricPrompt.AUTHENTICATION_RESULT_TYPE_BIOMETRIC) {
                retire(BiometricCancellation.CANCEL); return
            }
            try { crypto.execute {
                val outcome = try {
                    beforeProvider?.invoke()
                    if (!retired.get() && writer.finish(cipher, wrapped)) BiometricPromptOutcome.WRITTEN
                    else BiometricPromptOutcome.CANCELLED
                } catch (_: Exception) { BiometricPromptOutcome.FAILED }
                finally { wrapped.fill(0) }
                this.result.complete(outcome)
            } } catch (failure: RuntimeException) {
                queued.set(false)
                retire(BiometricCancellation.CANCEL)
                throw failure
            }
        }
        override fun retire(reason: BiometricCancellation) {
            if (!retired.compareAndSet(false, true)) return
            try { call.retireInput() }
            finally {
                writer.close()
                if (!queued.get()) wrapped.fill(0)
                result.complete(BiometricPromptOutcome.CANCELLED)
                main.post { coordinator.cancel(this, reason) }
            }
        }
        override fun promptDismissed() { /* Exact provider completion still belongs to the private executor. */ }
        fun dispose() {
            writer.close()
            if (!queued.get()) wrapped.fill(0)
            if (!result.isDone) retire(BiometricCancellation.CANCEL)
        }
    }

    private fun validate(binding: BiometricBindingView, attempt: String) {
        for (digest in listOf(binding.scopeDigest, binding.namespace, binding.workingCopy, binding.root, attempt))
            require(digest.matches(Regex("[0-9a-f]{64}")))
        require(binding.database.isNotEmpty() && binding.database.toByteArray(Charsets.UTF_8).size <= 128)
    }
    private fun sameBinding(binding: BiometricBindingView, spec: BiometricInputSpec) =
        binding.namespace == spec.namespace && binding.workingCopy == spec.workingCopy &&
            binding.database == spec.database && binding.root == spec.root && binding.epoch == spec.epoch
    private fun text(resource: Int): String {
        val configuration = android.content.res.Configuration(context.resources.configuration).apply {
            setLocale(java.util.Locale.forLanguageTag((context as TaypeerApplication).uiPreferences.language.tag))
        }
        return context.createConfigurationContext(configuration).getString(resource)
    }
    private fun unavailable(): Nothing = throw AndroidException.Biometry(BiometryFailure.UNAVAILABLE)
    private inline fun <T> guarded(action: () -> T): T = try { action() }
        catch (failure: AndroidException) { throw failure }
        catch (_: Exception) { unavailable() }
}
