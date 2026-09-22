package dev.taypeer.platform

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.os.Process
import dev.taypeer.bridge.bridgeVersion

/** Private process boundary. Document commands are added only with descriptor persistence. */
class DocumentService : Service() {
    private var owner: IBinder? = null
    private val ownerDied = IBinder.DeathRecipient { Process.killProcess(Process.myPid()) }
    private val endpoint = object : IDocumentProcess.Stub() {
        @Synchronized
        override fun initialize(host: IBinder): Int {
            check(owner == null)
            host.linkToDeath(ownerDied, 0)
            owner = host
            // Resolves the packaged UniFFI/JNA library in the isolated UID.
            check(bridgeVersion() == 1u)
            return Process.myPid()
        }
        override fun terminate() { Process.killProcess(Process.myPid()) }
    }
    override fun onBind(intent: Intent): IBinder = endpoint
    override fun onUnbind(intent: Intent): Boolean {
        Process.killProcess(Process.myPid())
        return false
    }
}
