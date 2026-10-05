package dev.taypeer

import android.app.Notification
import android.app.NotificationManager
import android.app.job.JobScheduler
import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.ExchangeScheduler
import dev.taypeer.platform.ExchangeService
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.TimeUnit

/** Tests the real OS notification and foreground lifetime; no decrypted document is opened. */
@RunWith(AndroidJUnit4::class)
class ExchangeTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private fun until(seconds: Long = 10, predicate: () -> Boolean) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        while (!predicate() && System.nanoTime() < deadline) Thread.sleep(20)
        assertTrue(predicate())
    }
    @Test fun explicitSessionIsSilentAndStopsWhileBackgroundSchedulingIsIndependent() {
        val activity = instrumentation.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val manager = context.getSystemService(NotificationManager::class.java)
        val host = runBlocking { (context.applicationContext as TaypeerApplication).host.await() }
        try {
            ExchangeScheduler.cancel(context)
            ExchangeScheduler.startForeground(context)
            until { manager.activeNotifications.any { it.id == ExchangeService.NOTIFICATION } }
            val channel = manager.getNotificationChannel(ExchangeService.CHANNEL)
            assertEquals(NotificationManager.IMPORTANCE_LOW, channel.importance)
            assertNull(channel.sound)
            assertFalse(channel.shouldVibrate())
            val notification = manager.activeNotifications.single { it.id == ExchangeService.NOTIFICATION }.notification
            assertTrue(notification.flags and Notification.FLAG_ONGOING_EVENT != 0)
            until { host.exchangeView(null).running }
            ExchangeScheduler.stopForeground(context)
            until { manager.activeNotifications.none { it.id == ExchangeService.NOTIFICATION } }
            until { !host.exchangeView(null).running }
            ExchangeScheduler.schedule(context)
            val job = context.getSystemService(JobScheduler::class.java).allPendingJobs.single { it.id == 1701 }
            assertTrue(job.isPeriodic)
            assertTrue(job.isPersisted)
            assertTrue(context.getSystemService(JobScheduler::class.java).allPendingJobs.any { it.id == 1701 })
        } finally {
            ExchangeScheduler.stopForeground(context)
            ExchangeScheduler.cancel(context)
            instrumentation.runOnMainSync { activity.finish() }
        }
    }
    @Test fun backgroundJobExchangesOnlyWithinItsBoundedLifetime() {
        val host = runBlocking { (context.applicationContext as TaypeerApplication).host.await() }
        try {
            ExchangeScheduler.stopForeground(context)
            ExchangeScheduler.schedule(context)
            instrumentation.uiAutomation.executeShellCommand("cmd jobscheduler run -f dev.taypeer 1701").use { descriptor ->
                android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { it.readBytes() }
            }
            until { host.exchangeView(null).running }
            until(30) { !host.exchangeView(null).running }
            assertTrue(context.getSystemService(NotificationManager::class.java).activeNotifications.none { it.id == ExchangeService.NOTIFICATION })
        } finally { ExchangeScheduler.cancel(context) }
    }
    @Test fun finishingBackgroundJobPreservesTheExplicitForegroundSession() {
        val activity = instrumentation.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val host = runBlocking { (context.applicationContext as TaypeerApplication).host.await() }
        val notifications = context.getSystemService(NotificationManager::class.java)
        try {
            ExchangeScheduler.cancel(context)
            ExchangeScheduler.stopForeground(context)
            until { !host.exchangeView(null).running }
            ExchangeScheduler.startForeground(context)
            until { host.exchangeView(null).running }
            ExchangeScheduler.schedule(context)
            instrumentation.uiAutomation.executeShellCommand("cmd jobscheduler run -f dev.taypeer 1701").use { descriptor ->
                android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { it.readBytes() }
            }
            Thread.sleep(22000) // The real background job relinquishes its 20-second lease.
            assertTrue(host.exchangeView(null).running)
            assertTrue(notifications.activeNotifications.any { it.id == ExchangeService.NOTIFICATION })
            ExchangeScheduler.stopForeground(context)
            until { !host.exchangeView(null).running }
        } finally {
            ExchangeScheduler.stopForeground(context)
            ExchangeScheduler.cancel(context)
            instrumentation.runOnMainSync { activity.finish() }
        }
    }
}
