package dev.taypeer.platform

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.os.Process
import dev.taypeer.bridge.bridgeVersion

/** Independent native-loading smoke, deliberately excluded from the release manifest. */
class LifecycleProbeService : Service() {
    private var owner: IBinder? = null
    private val died = IBinder.DeathRecipient { Process.killProcess(Process.myPid()) }
    override fun onBind(intent: Intent): IBinder = object : ILifecycleProbe.Stub() {
        @Synchronized override fun initialize(host: IBinder): Int {
            check(owner == null)
            host.linkToDeath(died, 0)
            owner = host
            check(bridgeVersion() == 2u)
            return Process.myPid()
        }
        override fun terminate() { Process.killProcess(Process.myPid()) }
    }
    override fun onUnbind(intent: Intent): Boolean { Process.killProcess(Process.myPid()); return false }
}
