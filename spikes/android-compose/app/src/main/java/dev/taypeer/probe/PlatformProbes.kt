package dev.taypeer.probe

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.os.SystemClock
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import androidx.activity.result.contract.ActivityResultContracts
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** OS integration exercises on public synthetic data only; no vault format. */
class PlatformProbes(private val activity: MainActivity, private val report: (Int) -> Unit) {
    private val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    private val handler = Handler(Looper.getMainLooper())
    private var ownership: Ownership? = null
    private data class Ownership(val marker: String, val digest: ByteArray, val deadline: Long)

    private val importer = activity.registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) { report(R.string.cancelled); return@registerForActivityResult }
        activity.lifecycleScope.launch {
            val ok = withContext(Dispatchers.IO) {
                runCatching {
                    val bytes = activity.contentResolver.openInputStream(uri)?.use {
                        val output = java.io.ByteArrayOutputStream()
                        val buffer = ByteArray(8192)
                        while (output.size() <= 1024 * 1024) {
                            val count = it.read(buffer, 0, minOf(buffer.size, 1024 * 1024 + 1 - output.size()))
                            if (count < 0) break
                            output.write(buffer, 0, count)
                        }
                        output.toByteArray()
                    } ?: error("unavailable")
                    require(bytes.size <= 1024 * 1024)
                    require(bytes.toString(Charsets.UTF_8).startsWith("SYNTHETIC-ONLY"))
                    val file = AtomicFile(File(activity.filesDir, "synthetic-working-copy.txt"))
                    val stream = file.startWrite()
                    try {
                        stream.write(bytes)
                        stream.flush()
                        stream.fd.sync()
                        file.finishWrite(stream)
                    } catch (error: Exception) {
                        file.failWrite(stream)
                        throw error
                    }
                }.isSuccess
            }
            report(if (ok) R.string.import_ok else R.string.error)
        }
    }

    private val exporter = activity.registerForActivityResult(ActivityResultContracts.CreateDocument("text/plain")) { uri ->
        if (uri == null) { report(R.string.cancelled); return@registerForActivityResult }
        activity.lifecycleScope.launch {
            val ok = withContext(Dispatchers.IO) {
                runCatching {
                    // Explicit export of a public fixture, never arbitrary Rust/UI secret state.
                    val descriptor = activity.contentResolver.openFileDescriptor(uri, "wt")
                        ?: error("unavailable")
                    android.os.ParcelFileDescriptor.AutoCloseOutputStream(descriptor).use {
                            it.write("SYNTHETIC-ONLY-Жук-42\n".toByteArray())
                            it.flush()
                            it.fd.sync()
                    }
                }.isSuccess
            }
            report(if (ok) R.string.exported else R.string.error)
        }
    }

    fun importSample() { importer.launch(arrayOf("text/plain", "application/octet-stream")) }
    fun exportSample() { exporter.launch("taypeer-synthetic-probe.txt") }

    fun copy(text: String) {
        val marker = "taypeer-probe:${UUID.randomUUID()}"
        val clip = ClipData.newPlainText(marker, text)
        clip.description.extras = PersistableBundle().apply {
            putBoolean("android.content.extra.IS_SENSITIVE", true)
        }
        try {
            clipboard.setPrimaryClip(clip)
            ownership = Ownership(marker, digest(text), SystemClock.elapsedRealtime() + 30_000)
            handler.postDelayed({ retryClipboardClear() }, 30_000)
            report(R.string.copied)
        } catch (_: Exception) { report(R.string.error) }
    }

    fun retryClipboardClear() {
        val owner = ownership ?: return
        if (SystemClock.elapsedRealtime() < owner.deadline || !activity.hasWindowFocus()) return
        // A null/unavailable read must never cause blind clearing. Retry when focused.
        val clip = try { clipboard.primaryClip } catch (_: Exception) { null } ?: return
        val text = if (clip.itemCount > 0) clip.getItemAt(0).text?.toString() else null
        if (clip.description.label?.toString() == owner.marker && text != null &&
            MessageDigest.isEqual(owner.digest, digest(text))) {
            try { clipboard.clearPrimaryClip() } catch (_: Exception) { return }
        }
        ownership = null
    }

    private fun digest(text: String) = MessageDigest.getInstance("SHA-256").digest(text.toByteArray())

    fun authenticate(localized: Context) {
        if (BiometricManager.from(activity).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            != BiometricManager.BIOMETRIC_SUCCESS) {
            report(R.string.biometric_unavailable)
            return
        }
        try {
            val alias = "taypeer-ui-probe-synthetic-key"
            val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            val key = if (store.containsAlias(alias)) store.getKey(alias, null) as SecretKey else {
                KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
                    init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                        .setUserAuthenticationRequired(true)
                        .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                        .setInvalidatedByBiometricEnrollment(true).build())
                }.generateKey()
            }
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key) }
            val prompt = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        val ok = runCatching {
                            val output = result.cryptoObject?.cipher?.doFinal("SYNTHETIC-ONLY".toByteArray())
                            require(output != null)
                            output.fill(0)
                        }.isSuccess
                        report(if (ok) R.string.biometric_ok else R.string.error)
                    }
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        report(R.string.biometric_unavailable)
                    }
                })
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(localized.getString(R.string.biometric_title))
                .setNegativeButtonText(localized.getString(R.string.close))
                .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG).build()
            prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
        } catch (_: Exception) { report(R.string.biometric_unavailable) }
    }
}
