package dev.taypeer

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Binder
import android.os.IBinder
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.platform.LifecycleProbeService
import dev.taypeer.platform.ILifecycleProbe
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class PlatformTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test fun encryptedCredentialsSurviveRecreationAndRejectCorruption() {
        val service = "PUBLIC-fixture-${UUID.randomUUID()}"
        val account = "PUBLIC-account"
        val source = "PUBLIC-test-credential".toByteArray()
        val store = KeystoreCredentials(context)
        assertNull(store.get(service, account))
        store.set(service, account, source.copyOf())
        assertArrayEquals(source, KeystoreCredentials(context).get(service, account))
        assertNull(store.get(service, "PUBLIC-other"))
        val directory = File(context.noBackupFilesDir, "credentials")
        val key = java.nio.ByteBuffer.allocate(8 + service.toByteArray().size + account.toByteArray().size)
            .putInt(service.toByteArray().size).put(service.toByteArray())
            .putInt(account.toByteArray().size).put(account.toByteArray()).array()
        val filename = java.security.MessageDigest.getInstance("SHA-256").digest(key)
            .joinToString("") { "%02x".format(it) } + ".credential"
        val file = File(directory, filename)
        val encoded = file.readBytes()
        assertFalse(encoded.toString(Charsets.ISO_8859_1).contains("PUBLIC-test-credential"))
        encoded[encoded.lastIndex] = (encoded.last().toInt() xor 1).toByte()
        file.writeBytes(encoded)
        assertThrows(AndroidException.Credentials::class.java) { store.get(service, account) }
        assertTrue(file.delete())
    }

    @Test fun rustHostUsesKeystoreCallbacksAndEnforcesSingleOwner() {
        val directory = File(context.noBackupFilesDir, "PUBLIC-profile-${UUID.randomUUID()}")
        val credentials = KeystoreCredentials(context)
        try {
            dev.taypeer.bridge.Host(directory.path, credentials).use { host ->
                host.activity()
                host.background()
                assertThrows(AndroidException.ProfileBusy::class.java) {
                    dev.taypeer.bridge.Host(directory.path, credentials)
                }
            }
            dev.taypeer.bridge.Host(directory.path, KeystoreCredentials(context)).use { it.background() }
        } finally { assertTrue(directory.deleteRecursively()) }
    }

    @Test fun isolatedInstancesLoadNativeAndTerminateIndependently() {
        val first = BoundProcess()
        val second = BoundProcess()
        try {
            first.bind(); second.bind()
            assertNotEquals(first.pid, second.pid)
            assertNotEquals(android.os.Process.myPid(), first.pid)
            first.remote.terminate()
            assertTrue(first.died.await(2, TimeUnit.SECONDS))
            assertTrue(second.binder.isBinderAlive)
            second.remote.terminate()
            assertTrue(second.died.await(2, TimeUnit.SECONDS))
        } finally { first.close(); second.close() }
    }

    private inner class BoundProcess : ServiceConnection {
        private val executor = Executors.newSingleThreadExecutor()
        private val connected = CountDownLatch(1)
        val died = CountDownLatch(1)
        lateinit var binder: IBinder
        lateinit var remote: ILifecycleProbe
        var pid = 0
        private var bound = false
        private val owner = Binder()
        fun bind() {
            bound = context.bindIsolatedService(Intent(context, LifecycleProbeService::class.java),
                Context.BIND_AUTO_CREATE, "generation_${UUID.randomUUID().toString().replace("-", "")}", executor, this)
            assertTrue(bound)
            assertTrue(connected.await(10, TimeUnit.SECONDS))
            pid = remote.initialize(owner)
        }
        override fun onServiceConnected(name: ComponentName, service: IBinder) {
            binder = service
            remote = ILifecycleProbe.Stub.asInterface(service)
            service.linkToDeath({ died.countDown() }, 0)
            connected.countDown()
        }
        override fun onServiceDisconnected(name: ComponentName) { died.countDown() }
        fun close() {
            if (bound) { context.unbindService(this); bound = false }
            executor.shutdownNow()
        }
    }
}
