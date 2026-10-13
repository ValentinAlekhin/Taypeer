package dev.taypeer.qr

import android.os.SystemClock
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import java.util.concurrent.Executor
import java.util.concurrent.atomic.AtomicBoolean

/** Owns frame delivery and releases every camera buffer, including stale frames. */
internal class QrFrameAnalyzer(
    private val completionExecutor: Executor,
    private val canDeliver: () -> Boolean,
    private val onCode: (String) -> Unit,
    private val clock: () -> Long = SystemClock::elapsedRealtime,
) : ImageAnalysis.Analyzer {
    private val active = AtomicBoolean(true)
    private val delivered = AtomicBoolean(false)
    private val decoder = QrDecoder()
    private var lastFrame = Long.MIN_VALUE

    override fun analyze(frame: ImageProxy) {
        try {
            val now = clock()
            if (!active.get() || delivered.get() || (lastFrame != Long.MIN_VALUE && now - lastFrame < 200)) return
            lastFrame = now
            val plane = frame.planes.firstOrNull() ?: return
            val bytes = QrDecoder.luminance(plane.buffer, frame.width, frame.height, plane.rowStride, plane.pixelStride) ?: return
            val code = decoder.decode(bytes, frame.width, frame.height) ?: return
            if (delivered.compareAndSet(false, true)) completionExecutor.execute {
                if (active.get() && canDeliver()) onCode(code) else delivered.set(false)
            }
        } finally { frame.close() }
    }

    fun stop() { active.set(false) }
}
