package dev.taypeer

import android.app.Application
import androidx.test.runner.AndroidJUnitRunner
import dev.taypeer.bridge.RelayPreference
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext

/** Tests never depend on public relays; private network fixtures choose their own transport. */
class TaypeerTestRunner : AndroidJUnitRunner() {
    override fun callApplicationOnCreate(application: Application) {
        super.callApplicationOnCreate(application)
        if (application is TaypeerApplication) runBlocking {
            withContext(Dispatchers.IO) { application.host.await().setRelayPreference(RelayPreference.Disabled) }
        }
    }
}
