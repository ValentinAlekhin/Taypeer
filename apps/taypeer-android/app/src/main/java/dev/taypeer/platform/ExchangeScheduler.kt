package dev.taypeer.platform

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import dev.taypeer.TaypeerApplication
import kotlinx.coroutines.runBlocking
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/** Android only schedules lifetimes; verified exchange and retry rules stay in the common host. */
object ExchangeScheduler {
    private const val JOB = 1701
    internal const val READY_REQUEST = "exchange_ready_request"
    private val pending = ConcurrentHashMap<String, (Boolean) -> Unit>()
    /** Ready is delivered on the main thread after native start and wake, without document data. */
    fun startForeground(context: Context, ready: ((Boolean) -> Unit)? = null) {
        val request = ready?.let { UUID.randomUUID().toString().also { id -> pending[id] = it } }
        try {
            context.startForegroundService(Intent(context, ExchangeService::class.java).apply {
                request?.let { putExtra(READY_REQUEST, it) }
            })
        } catch (error: Exception) {
            request?.let(pending::remove)
            throw error
        }
    }
    internal fun ready(request: String, success: Boolean) { pending.remove(request)?.invoke(success) }
    fun stopForeground(context: Context) {
        val service = Intent(context, ExchangeService::class.java)
        context.stopService(service)
    }
    fun schedule(context: Context) {
        val scheduler = context.getSystemService(JobScheduler::class.java)
        val job = JobInfo.Builder(JOB, ComponentName(context, ExchangeJobService::class.java))
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
            .setPeriodic(15 * 60 * 1000L)
            .setPersisted(true)
            .build()
        check(scheduler.schedule(job) == JobScheduler.RESULT_SUCCESS)
    }
    fun cancel(context: Context) {
        context.getSystemService(JobScheduler::class.java).cancel(JOB)
    }
}

/** Serialize network ownership so a finishing background job cannot stop an explicit session. */
internal object ExchangeLifetime {
    private val executor = Executors.newSingleThreadExecutor()
    private val owners = mutableSetOf<String>()
    fun acquire(context: Context, owner: String, ready: (Boolean) -> Unit) {
        val application = context.applicationContext as TaypeerApplication
        executor.execute {
            try {
                val host = runBlocking { application.host.await() }
                if (owners.isEmpty()) host.startExchange()
                owners.add(owner)
                host.wakeExchange()
                ready(true)
            } catch (_: Exception) {
                owners.remove(owner)
                if (owners.isEmpty()) {
                    try { runBlocking { application.host.await() }.stopExchange() } catch (_: Exception) {}
                }
                ready(false)
            }
        }
    }
    fun release(context: Context, owner: String) {
        val application = context.applicationContext as TaypeerApplication
        executor.execute {
            if (owners.remove(owner) && owners.isEmpty()) {
                try { runBlocking { application.host.await() }.stopExchange() } catch (_: Exception) {}
            }
        }
    }
}
