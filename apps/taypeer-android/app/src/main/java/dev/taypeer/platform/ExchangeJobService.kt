package dev.taypeer.platform

import android.app.job.JobParameters
import android.app.job.JobService
import android.os.Handler
import android.os.Looper
import java.util.UUID

/** OS wakeup has a bounded ciphertext-only lifetime and never authenticates a document. */
class ExchangeJobService : JobService() {
    private val handler = Handler(Looper.getMainLooper())
    private data class Work(val owner: String, val finish: Runnable)
    private val active = mutableMapOf<Int, Work>()
    override fun onStartJob(parameters: JobParameters): Boolean {
        active.remove(parameters.jobId)?.let {
            handler.removeCallbacks(it.finish)
            ExchangeLifetime.release(this, it.owner)
        }
        val owner = "job:${UUID.randomUUID()}"
        val finish = Runnable {
            if (active[parameters.jobId]?.owner == owner) {
                active.remove(parameters.jobId)
                ExchangeLifetime.release(this, owner)
                jobFinished(parameters, false)
            }
        }
        active[parameters.jobId] = Work(owner, finish)
        handler.postDelayed(finish, 20_000)
        ExchangeLifetime.acquire(this, owner) { success ->
            if (!success) handler.post(finish)
        }
        return true
    }
    override fun onStopJob(parameters: JobParameters): Boolean {
        active.remove(parameters.jobId)?.let {
            handler.removeCallbacks(it.finish)
            ExchangeLifetime.release(this, it.owner)
        }
        return true
    }
    override fun onDestroy() {
        active.values.forEach {
            handler.removeCallbacks(it.finish)
            ExchangeLifetime.release(this, it.owner)
        }
        active.clear()
        super.onDestroy()
    }
}
