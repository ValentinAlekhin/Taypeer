package dev.taypeer.platform

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.os.Process
import dev.taypeer.bridge.Host
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Real document host fixture: OS death must terminate the isolated authenticated worker. */
class HostDeathProbeService : Service() {
    private val connected = CountDownLatch(1)
    private val executor = Executors.newSingleThreadExecutor()
    private var child: IBinder? = null
    private var host: Host? = null
    private var manager: DocumentManager? = null
    override fun onCreate() {
        super.onCreate()
        executor.execute {
            try {
                val profile = File(noBackupFilesDir, "PUBLIC-host-death-${UUID.randomUUID()}")
                val host = Host(profile.path, KeystoreCredentials(this))
                this.host = host
                val manager = DocumentManager(this, host)
                this.manager = manager
                val session = manager.create("PUBLIC host death", null, "PUBLIC creation", "PUBLIC fixture master")
                check(session.overview("").name == "PUBLIC host death")
                child = manager.process(session).binder
            } catch (_: Exception) { /* The acceptance caller observes a definite setup failure. */ }
            finally { connected.countDown() }
        }
    }
    private val endpoint = object : IHostDeathProbe.Stub() {
        override fun child(): IBinder {
            check(connected.await(20, TimeUnit.SECONDS))
            return checkNotNull(child)
        }
        override fun exit() { Process.killProcess(Process.myPid()) }
    }
    override fun onBind(intent: Intent): IBinder = endpoint
    override fun onDestroy() {
        manager?.background()
        executor.execute { try { manager?.close() } finally { host?.close(); executor.shutdown() } }
        super.onDestroy()
    }
}
