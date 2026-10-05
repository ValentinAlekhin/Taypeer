package dev.taypeer.platform

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import dev.taypeer.MainActivity
import dev.taypeer.R
import java.util.UUID

/** Explicit, bounded encrypted exchange. The mandatory notification contains no document data. */
class ExchangeService : Service() {
    private val handler = Handler(Looper.getMainLooper())
    private val owner = "foreground:${UUID.randomUUID()}"
    private val finish = Runnable { stopSelf() }
    private var started = false
    private var destroyed = false
    private var ready: Boolean? = null
    private val requests = mutableListOf<String>()
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        intent?.getStringExtra(ExchangeScheduler.READY_REQUEST)?.let { request ->
            ready?.let { ExchangeScheduler.ready(request, it) } ?: requests.add(request)
        }
        if (!started) {
            started = true
            startForeground(NOTIFICATION, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
            handler.postDelayed(finish, 15 * 60 * 1000L)
            ExchangeLifetime.acquire(this, owner) { success -> handler.post {
                if (!destroyed) {
                    ready = success
                    requests.forEach { ExchangeScheduler.ready(it, success) }
                    requests.clear()
                    if (!success) stopSelf()
                }
            } }
        }
        return START_NOT_STICKY
    }
    override fun onDestroy() {
        destroyed = true
        requests.forEach { ExchangeScheduler.ready(it, false) }
        requests.clear()
        handler.removeCallbacks(finish)
        ExchangeLifetime.release(this, owner)
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }
    override fun onTimeout(startId: Int, fgsType: Int) { stopSelf(startId) }
    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.exchange_channel), NotificationManager.IMPORTANCE_LOW).apply {
            setSound(null, null)
            enableVibration(false)
            setShowBadge(false)
        })
        val destination = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle(getString(R.string.exchange_notification))
            .setContentText(getString(R.string.exchange_notification_detail))
            .setContentIntent(destination)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .build()
    }
    companion object {
        internal const val CHANNEL = "encrypted-exchange"
        internal const val NOTIFICATION = 1702
    }
}
