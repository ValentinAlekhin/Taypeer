package dev.taypeer.platform

import android.app.Service
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Binder
import android.os.IBinder
import android.os.Process
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Debug fixture proving that OS/Binder host death reaches an isolated process. */
class HostDeathProbeService : Service() {
    private val connected = CountDownLatch(1)
    private val executor = Executors.newSingleThreadExecutor()
    private val owner = Binder()
    private var child: IBinder? = null
    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            IDocumentProcess.Stub.asInterface(binder).initialize(owner)
            child = binder
            connected.countDown()
        }
        override fun onServiceDisconnected(name: ComponentName) { child = null }
    }
    override fun onCreate() {
        super.onCreate()
        check(bindIsolatedService(Intent(this, DocumentService::class.java), Context.BIND_AUTO_CREATE,
            "host_death_${UUID.randomUUID().toString().replace("-", "")}", executor, connection))
    }
    private val endpoint = object : IHostDeathProbe.Stub() {
        override fun child(): IBinder {
            check(connected.await(10, TimeUnit.SECONDS))
            return checkNotNull(child)
        }
        override fun exit() { Process.killProcess(Process.myPid()) }
    }
    override fun onBind(intent: Intent): IBinder = endpoint
    override fun onDestroy() { unbindService(connection); executor.shutdownNow(); super.onDestroy() }
}
