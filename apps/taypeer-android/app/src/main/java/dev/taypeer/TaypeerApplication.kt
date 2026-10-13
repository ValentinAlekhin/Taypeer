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
    val uiPreferences by lazy { UiPreferences(this) }
    val clipboard by lazy { dev.taypeer.platform.SensitiveClipboard(this) }
    internal val secretAccess by lazy { SecretAccessState() }
    internal val deviceSettings by lazy {
        DeviceSettingsState(host, CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate))
    }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    internal val osBiometry by lazy {
        check(Application.getProcessName() == packageName)
        dev.taypeer.platform.AndroidOsBiometry(this)
    }
    val host by lazy {
        check(Application.getProcessName() == packageName)
        scope.async { Host.withOsBiometry(File(noBackupFilesDir, "profile").path,
            KeystoreCredentials(this@TaypeerApplication), osBiometry) }
    }
}
