package dev.taypeer.qr

import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.nio.ByteBuffer

/** Local image decoding only. Rust validates the invitation when the user submits it. */
internal class QrDecoder {
    private val reader = QRCodeReader()
    private val hints = mapOf(DecodeHintType.TRY_HARDER to true, DecodeHintType.CHARACTER_SET to "UTF-8")

    fun decode(luminance: ByteArray, width: Int, height: Int): String? {
        if (width <= 0 || height <= 0 || width.toLong() * height > MAX_PIXELS || luminance.size.toLong() != width.toLong() * height) return null
        var pixels = luminance
        var columns = width
        var rows = height
        repeat(4) { orientation ->
            try {
                val source = PlanarYUVLuminanceSource(pixels, columns, rows, 0, 0, columns, rows, false)
                val result = reader.decode(BinaryBitmap(HybridBinarizer(source)), hints).text
                if (result.isNotEmpty() && result.length <= MAX_TEXT) return result
            } catch (_: ReaderException) {
                // Dense invitations are not detected equally well at every sensor orientation.
            } finally { reader.reset() }
            if (orientation < 3) {
                val before = pixels
                pixels = ByteArray(before.size) { index -> before[(rows - 1 - index % rows) * columns + index / rows] }
                val swap = columns; columns = rows; rows = swap
            }
        }
        return null
    }

    companion object {
        const val MAX_PIXELS = 4096 * 2160
        const val MAX_TEXT = 4096

        /** Y planes can have padding, a nonzero buffer position, and non-unit pixel stride. */
        fun luminance(buffer: ByteBuffer, width: Int, height: Int, rowStride: Int, pixelStride: Int): ByteArray? {
            val count = width.toLong() * height
            if (width <= 0 || height <= 0 || count > MAX_PIXELS || pixelStride <= 0 || rowStride <= 0) return null
            val lastInRow = (width - 1L) * pixelStride
            if (lastInRow >= rowStride) return null
            val source = buffer.duplicate()
            val start = source.position()
            val last = start.toLong() + (height - 1L) * rowStride + lastInRow
            if (last >= source.limit()) return null
            return ByteArray(count.toInt()).also { pixels ->
                for (y in 0 until height) for (x in 0 until width) {
                    pixels[y * width + x] = source.get(start + y * rowStride + x * pixelStride)
                }
            }
        }
    }
}
