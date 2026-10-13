package dev.taypeer.qr

import androidx.camera.core.ImageProxy
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.*
import org.junit.Test
import java.lang.reflect.Proxy
import java.nio.ByteBuffer
import java.util.concurrent.Executor

class QrFrameAnalyzerTest {
    private class Frame(readable: Boolean = true) {
        var closed = 0
        private val matrix = QRCodeWriter().encode("PUBLIC camera invitation", BarcodeFormat.QR_CODE, 320, 320)
        private val buffer = ByteBuffer.wrap(ByteArray(320 * 320) { if (readable && matrix[it % 320, it / 320]) 0 else 255.toByte() })
        private val plane = Proxy.newProxyInstance(ImageProxy.PlaneProxy::class.java.classLoader, arrayOf(ImageProxy.PlaneProxy::class.java)) { _, method, _ ->
            when (method.name) { "getBuffer" -> buffer; "getRowStride" -> 320; "getPixelStride" -> 1; else -> null }
        } as ImageProxy.PlaneProxy
        val proxy = Proxy.newProxyInstance(ImageProxy::class.java.classLoader, arrayOf(ImageProxy::class.java)) { _, method, _ ->
            when (method.name) { "getWidth", "getHeight" -> 320; "getPlanes" -> arrayOf(plane); "close" -> { closed++; null }; else -> null }
        } as ImageProxy
    }
    private class Queued : Executor {
        val tasks = mutableListOf<Runnable>()
        override fun execute(command: Runnable) { tasks += command }
        fun run() { tasks.toList().also { tasks.clear() }.forEach { it.run() } }
    }

    @Test fun releasesEveryFrameAndDeliversReadableCodeOnce() {
        val queue = Queued()
        var now = 0L
        val results = mutableListOf<String>()
        val analyzer = QrFrameAnalyzer(queue, { true }, results::add, { now })
        val blank = Frame(false); analyzer.analyze(blank.proxy); assertEquals(1, blank.closed)
        now = 250
        val readable = Frame(); analyzer.analyze(readable.proxy); assertEquals(1, readable.closed)
        val duplicate = Frame(); analyzer.analyze(duplicate.proxy); assertEquals(1, duplicate.closed)
        assertTrue(results.isEmpty())
        queue.run()
        assertEquals(listOf("PUBLIC camera invitation"), results)
    }

    @Test fun closeSuppressesAlreadyQueuedCameraCompletion() {
        val queue = Queued()
        val results = mutableListOf<String>()
        val analyzer = QrFrameAnalyzer(queue, { true }, results::add, { 0 })
        val frame = Frame(); analyzer.analyze(frame.proxy)
        analyzer.stop(); queue.run()
        assertTrue(results.isEmpty()); assertEquals(1, frame.closed)
        val stale = Frame(); analyzer.analyze(stale.proxy); assertEquals(1, stale.closed)
        assertTrue(queue.tasks.isEmpty())
    }

    @Test fun pausedCompletionIsDiscardedAndResumeCanScanAgain() {
        val queue = Queued()
        var resumed = false
        var now = 0L
        val results = mutableListOf<String>()
        val analyzer = QrFrameAnalyzer(queue, { resumed }, results::add, { now })
        analyzer.analyze(Frame().proxy); queue.run(); assertTrue(results.isEmpty())
        resumed = true; now = 250
        analyzer.analyze(Frame().proxy); queue.run()
        assertEquals(listOf("PUBLIC camera invitation"), results)
    }
}
