package dev.taypeer

import android.app.Application
import dev.taypeer.bridge.Host
import dev.taypeer.platform.KeystoreCredentials
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import java.io.File

/** One host owner per process. Isolated workers must not initialize host credentials. */
class TaypeerApplication : Application() {
    val clipboard by lazy { dev.taypeer.platform.SensitiveClipboard(this) }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    val host by lazy {
        check(Application.getProcessName() == packageName)
        scope.async { Host(File(noBackupFilesDir, "profile").path, KeystoreCredentials(this@TaypeerApplication)) }
    }
}
