package dev.taypeer.biometricspike

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.Message
import android.os.Messenger
import android.os.ParcelFileDescriptor
import android.os.Process
import android.system.Os
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import org.junit.Assert.*

/** Fixed PUBLIC-pattern validation in the actual isolated Native entry, never a byte getter. */
internal object PublicIsolatedNativeProbe {
    /** Borrow the sender's PFD until the Native result; its caller retains close ownership. */
    fun verify(original: ParcelFileDescriptor, expectedExact: Boolean, expectedFailure: String = "none") {
        check(Looper.myLooper() != Looper.getMainLooper()) { "PUBLIC isolated probe must wait off Main" }
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val connected = CompletableFuture<Messenger>()
        val completed = CompletableFuture<Bundle>()
        val reply = Messenger(Handler(Looper.getMainLooper()) { message ->
            completed.complete(message.data)
            true
        })
        val connection = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName, service: IBinder) {
                connected.complete(Messenger(service))
            }
            private fun disconnected() {
                val failure = AssertionError("PUBLIC isolated Native service disconnected")
                connected.completeExceptionally(failure)
                completed.completeExceptionally(failure)
            }
            override fun onServiceDisconnected(name: ComponentName) = disconnected()
            override fun onBindingDied(name: ComponentName) = disconnected()
            override fun onNullBinding(name: ComponentName) = disconnected()
        }
        assertTrue(context.bindService(Intent(context, PublicIsolatedPipeService::class.java), connection, Context.BIND_AUTO_CREATE))
        try {
            Os.fstat(original.fileDescriptor)
            connected.get(5, TimeUnit.SECONDS).send(Message.obtain().apply {
                replyTo = reply
                data = Bundle().apply { putParcelable("PUBLIC-pipe", original) }
            })
            val result = completed.get(5, TimeUnit.SECONDS)
            val keys = setOf("PUBLIC-exact", "PUBLIC-uid", "PUBLIC-original-held", "PUBLIC-stage", "PUBLIC-failure")
            assertTrue("The isolated Native response must contain every public result field", result.keySet().containsAll(keys))
            val diagnostic = "PUBLIC isolated Native diagnostic: stage=${result.getString("PUBLIC-stage")}, failure=${result.getString("PUBLIC-failure")}"
            assertEquals(diagnostic, expectedExact, result.getBoolean("PUBLIC-exact"))
            assertEquals(diagnostic, expectedFailure, result.getString("PUBLIC-failure"))
            assertEquals(diagnostic, if (expectedFailure == "none") "native-complete" else "native-import", result.getString("PUBLIC-stage"))
            assertTrue("The receiver must report a real positive UID", result.getInt("PUBLIC-uid") > 0)
            assertNotEquals("The receiver must have a separate real isolated UID", Process.myUid(), result.getInt("PUBLIC-uid"))
            assertTrue("Native duplication/read/refusal must preserve its held Binder PFD", result.getBoolean("PUBLIC-original-held"))
            // The receiver's native duplicate and service-owned PFD cannot close the sender's PFD.
            Os.fstat(original.fileDescriptor)
        } finally {
            context.unbindService(connection)
        }
    }
}
