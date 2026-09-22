package dev.taypeer.platform

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.os.SystemClock
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import java.security.MessageDigest
import java.util.UUID
import javax.crypto.KeyGenerator
import javax.crypto.Mac
import javax.crypto.SecretKey

/** Application-owned receipt contains an HMAC, never plaintext or a password hash. */
class SensitiveClipboard(context: Context) {
    private val clipboard = context.getSystemService(ClipboardManager::class.java)
    private val receipt = context.getSharedPreferences("clipboard-ownership", Context.MODE_PRIVATE)
    private val handler = Handler(Looper.getMainLooper())
    private var focused = false
    private val expire = Runnable { checkExpired() }

    fun copy(value: String, seconds: Long = 30) {
        require(seconds > 0 && seconds <= Long.MAX_VALUE / 1000)
        val now = System.currentTimeMillis()
        val deadline = Math.addExact(now, Math.multiplyExact(seconds, 1000))
        val token = UUID.randomUUID().toString()
        val mac = authenticate(value)
        val clip = ClipData.newPlainText("Taypeer", value).apply {
            description.extras = PersistableBundle().apply {
                putBoolean("android.content.extra.IS_SENSITIVE", true)
                putString(OWNER, token)
            }
        }
        // Receipt precedes publication: a crash cannot leave an untracked owned secret.
        check(receipt.edit().putString("token", token).putString("mac", mac)
            .putLong("deadline", deadline).putLong("started", SystemClock.elapsedRealtime())
            .putLong("duration", seconds * 1000).commit())
        clipboard.setPrimaryClip(clip)
        schedule(deadline)
    }

    fun focusChanged(value: Boolean) {
        focused = value
        if (focused) checkExpired()
    }
    private fun checkExpired() {
        val token = receipt.getString("token", null) ?: return
        val deadline = receipt.getLong("deadline", 0)
        val started = receipt.getLong("started", 0)
        val elapsed = SystemClock.elapsedRealtime()
        val remaining = receipt.getLong("duration", 0) - (elapsed - started)
        if (elapsed >= started && remaining > 0 && System.currentTimeMillis() < deadline) {
            handler.removeCallbacks(expire)
            handler.postDelayed(expire, minOf(remaining, deadline - System.currentTimeMillis()))
            return
        }
        if (!focused) return // Android may deny clipboard reads while unfocused.
        val clip = try { clipboard.primaryClip } catch (_: SecurityException) { return }
            ?: return // Unavailable is not evidence that the original value disappeared.
        val text = if (clip.itemCount == 1) clip.getItemAt(0).text?.toString() else null
        val owns = clip.description.extras?.getString(OWNER) == token && text != null &&
            MessageDigest.isEqual(authenticate(text).toByteArray(), receipt.getString("mac", "")!!.toByteArray())
        if (owns) clipboard.clearPrimaryClip()
        receipt.edit().clear().commit()
    }
    private fun schedule(deadline: Long) {
        handler.removeCallbacks(expire)
        handler.postDelayed(expire, (deadline - System.currentTimeMillis()).coerceAtLeast(0))
    }
    private fun authenticate(value: String): String {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val key = (store.getKey(KEY, null) as? SecretKey) ?: KeyGenerator
            .getInstance(KeyProperties.KEY_ALGORITHM_HMAC_SHA256, "AndroidKeyStore").run {
                init(KeyGenParameterSpec.Builder(KEY, KeyProperties.PURPOSE_SIGN).build())
                generateKey()
            }
        val bytes = value.toByteArray(Charsets.UTF_8)
        return try {
            Mac.getInstance("HmacSHA256").run { init(key); doFinal(bytes) }
                .joinToString("") { "%02x".format(it) }
        } finally { bytes.fill(0) }
    }
    companion object {
        private const val OWNER = "dev.taypeer.clipboard.owner"
        private const val KEY = "dev.taypeer.clipboard.mac.v1"
    }
}
