package dev.taypeer

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.IBinder
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.HostDeathProbeService
import dev.taypeer.platform.IHostDeathProbe
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class HostDeathTest {
    @Test fun isolatedProcessDiesWhenItsHostIsKilled() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val connected = CountDownLatch(1)
        val executor = Executors.newSingleThreadExecutor()
        var host: IHostDeathProbe? = null
        val connection = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName, service: IBinder) {
                host = IHostDeathProbe.Stub.asInterface(service)
                connected.countDown()
            }
            override fun onServiceDisconnected(name: ComponentName) = Unit
        }
        val bound = context.bindService(Intent(context, HostDeathProbeService::class.java),
            Context.BIND_AUTO_CREATE, executor, connection)
        try {
            assertTrue(bound)
            assertTrue(connected.await(10, TimeUnit.SECONDS))
            val worker = checkNotNull(host).child()
            val died = CountDownLatch(1)
            worker.linkToDeath({ died.countDown() }, 0)
            checkNotNull(host).exit()
            assertTrue(died.await(2, TimeUnit.SECONDS))
        } finally { if (bound) context.unbindService(connection); executor.shutdownNow() }
    }
}
