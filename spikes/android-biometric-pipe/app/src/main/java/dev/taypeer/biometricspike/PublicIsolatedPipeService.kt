package dev.taypeer.biometricspike

import android.app.Service
import android.content.Intent
import android.os.*
import android.system.Os
import dev.taypeer.bridge.verifyPublicBiometricPipe

/** Actual isolated Rust importer; it accepts only the fixed PUBLIC verification pattern. */
class PublicIsolatedPipeService : Service() {
    private val receiver = Messenger(Handler(Looper.getMainLooper()) { message ->
        @Suppress("DEPRECATION")
        val pipe = message.data.getParcelable<ParcelFileDescriptor>("PUBLIC-pipe")
        val reply = message.replyTo
        Thread({
            var valid = false
            var originalHeld = false
            var stage = "missing"
            var failureType = "none"
            pipe?.use { owned ->
                try {
                    stage = "native-import"
                    // Rust duplicates the live Binder grant directly with CLOEXEC. It owns
                    // only that duplicate and uses the actual bounded runtime key reader.
                    valid = verifyPublicBiometricPipe(owned.fd)
                    stage = "native-complete"
                } catch (failure: Exception) {
                    failureType = failure.javaClass.simpleName
                } finally {
                    originalHeld = try { Os.fstat(owned.fileDescriptor); true }
                        catch (_: android.system.ErrnoException) { false }
                }
            }
            reply?.send(Message.obtain().apply {
                data = Bundle().apply {
                    putBoolean("PUBLIC-exact", valid)
                    putInt("PUBLIC-uid", Process.myUid())
                    putBoolean("PUBLIC-original-held", originalHeld)
                    putString("PUBLIC-stage", stage)
                    putString("PUBLIC-failure", failureType)
                }
            })
        }, "PUBLIC-isolated-native-pipe-probe").start()
        true
    })
    override fun onBind(intent: Intent) = receiver.binder
}
