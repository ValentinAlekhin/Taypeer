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
class SensitiveClipboard internal constructor(context: Context,
    private val publish: (ClipboardManager, ClipData) -> Unit,
) {
    constructor(context: Context) : this(context, { manager, clip -> manager.setPrimaryClip(clip) })
    private val clipboard = context.getSystemService(ClipboardManager::class.java)
    private val receipt = context.getSharedPreferences("clipboard-ownership", Context.MODE_PRIVATE)
    private val handler = Handler(Looper.getMainLooper())
    private var focused = false
    // An OS/Keystore failure during a timer keeps the durable receipts for the next focus.
    private val expire = Runnable { try { checkExpired() } catch (_: Exception) { } }
    private data class Ownership(val token: String, val mac: String, val timed: Boolean,
        val deadline: Long, val started: Long, val duration: Long)

    /** The caller supplies the loaded Rust-validated policy; null disables our timer. */
    fun copy(value: String, seconds: UInt?) {
        val now = System.currentTimeMillis()
        val duration = seconds?.toLong()?.times(1000)
        val deadline = duration?.let { Math.addExact(now, it) }
        var previous = owners()
        if (previous.size == 2) {
            previous = reconcile(previous) ?: throw IllegalStateException("Clipboard ownership remains uncertain")
        }
        val candidate = Ownership(UUID.randomUUID().toString(), authenticate(value), duration != null,
            deadline ?: 0, SystemClock.elapsedRealtime(), duration ?: 0)
        val clip = ClipData.newPlainText("Taypeer", value).apply {
            description.extras = PersistableBundle().apply {
                putBoolean("android.content.extra.IS_SENSITIVE", true)
                putString(OWNER, candidate.token)
            }
        }
        // Persist both possible owners before publication; an uncertain reply cannot forget either.
        saveOwners(previous + candidate)
        try {
            try { publish(clipboard, clip) }
            catch (denied: SecurityException) {
                // A definite access denial retains the previous owner's original deadline.
                saveOwners(previous)
                throw denied
            }
            saveOwners(listOf(candidate))
        }
        finally { scheduleOwners() }
    }

    private fun owners(): List<Ownership> = listOfNotNull(owner(""), owner("candidate."))
    private fun owner(prefix: String): Ownership? {
        val token = receipt.getString("${prefix}token", null) ?: return null
        val mac = receipt.getString("${prefix}mac", null) ?: throw IllegalStateException("Invalid clipboard receipt")
        return Ownership(token, mac, receipt.getBoolean("${prefix}timed", true),
            receipt.getLong("${prefix}deadline", 0), receipt.getLong("${prefix}started", 0),
            receipt.getLong("${prefix}duration", 0)) // Legacy receipts were always timed.
    }
    private fun saveOwners(owners: List<Ownership>) {
        check(owners.size <= 2)
        val editor = receipt.edit().clear()
        owners.forEachIndexed { index, owner ->
            val prefix = if (index == 0) "" else "candidate."
            editor.putString("${prefix}token", owner.token).putString("${prefix}mac", owner.mac)
                .putBoolean("${prefix}timed", owner.timed).putLong("${prefix}deadline", owner.deadline)
                .putLong("${prefix}started", owner.started).putLong("${prefix}duration", owner.duration)
        }
        check(editor.commit())
    }

    fun focusChanged(value: Boolean) {
        focused = value
        if (focused) checkExpired()
    }
    private fun checkExpired() {
        val retained = owners()
        val current = reconcile(retained)
        if (current == null) {
            // Unavailable reads do not prove either value disappeared. Wait for focus after expiry.
            scheduleOwners(includeExpired = false)
            return
        }
        val owner = current.singleOrNull()
        if (owner != null && owner.timed && remaining(owner) <= 0) {
            clipboard.clearPrimaryClip()
            saveOwners(emptyList())
        }
        scheduleOwners(includeExpired = false)
    }
    /** Null means unavailable; an empty result means a readable foreign clip, never ours. */
    private fun reconcile(owners: List<Ownership>): List<Ownership>? {
        if (owners.isEmpty()) return emptyList()
        if (!focused) return null // Android may deny clipboard reads while unfocused.
        val clip = try { clipboard.primaryClip } catch (_: SecurityException) { return null }
            ?: return null
        val text = if (clip.itemCount == 1) clip.getItemAt(0).text?.toString() else null
        val owner = owners.firstOrNull { it.token == clip.description.extras?.getString(OWNER) }
        val matches = owner != null && text != null &&
            MessageDigest.isEqual(authenticate(text).toByteArray(), owner.mac.toByteArray())
        val current = if (matches) listOf(owner!!) else emptyList()
        if (current != owners) saveOwners(current)
        return current
    }
    private fun remaining(owner: Ownership): Long {
        val elapsed = SystemClock.elapsedRealtime()
        if (elapsed < owner.started) return 0 // Reboot invalidates the old monotonic deadline.
        return minOf(owner.duration - (elapsed - owner.started), owner.deadline - System.currentTimeMillis())
            .coerceAtLeast(0)
    }
    private fun scheduleOwners(includeExpired: Boolean = true) {
        handler.removeCallbacks(expire)
        val delay = owners().filter(Ownership::timed).map(::remaining)
            .filter { includeExpired || it > 0 }.minOrNull() ?: return
        handler.postDelayed(expire, delay)
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
