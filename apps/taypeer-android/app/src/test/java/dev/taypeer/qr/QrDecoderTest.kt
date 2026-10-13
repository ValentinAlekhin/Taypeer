package dev.taypeer.qr

import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.*
import org.junit.Test
import java.nio.ByteBuffer

class QrDecoderTest {
    private val code = "{\"invitation\":\"PUBLIC SYNTHETIC INVITATION\",\"route\":\"" + "0123456789abcdef".repeat(70) + "\"}"
    private fun image(text: String, size: Int = 640): ByteArray {
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size, mapOf(EncodeHintType.CHARACTER_SET to "UTF-8"))
        return ByteArray(size * size) { index -> if (matrix[index % size, index / size]) 0 else 255.toByte() }
    }

    @Test fun realQrRoundTripsLongInvitationExactly() {
        val decoder = QrDecoder()
        assertEquals(code, decoder.decode(image(code), 640, 640))
        val unicode = "PUBLIC приглашение · code with spaces  "
        assertEquals(unicode, decoder.decode(image(unicode), 640, 640))
    }

    @Test fun rotatedFramesDecodeAndUnreadableFramesDoNotPoisonNextScan() {
        var pixels = image(code)
        val decoder = QrDecoder()
        repeat(4) {
            assertEquals(code, decoder.decode(pixels, 640, 640))
            assertNull(decoder.decode(ByteArray(640 * 640) { 255.toByte() }, 640, 640))
            val before = pixels
            pixels = ByteArray(before.size) { index -> before[(639 - index % 640) * 640 + index / 640] }
        }
    }

    @Test fun paddedCameraPlaneWithPixelStrideAndBufferOffsetRoundTrips() {
        val size = 640
        val pixels = image(code)
        val rowStride = size * 2 + 24
        val buffer = ByteBuffer.allocate(7 + rowStride * size)
        for (y in 0 until size) for (x in 0 until size) buffer.put(7 + y * rowStride + x * 2, pixels[y * size + x])
        buffer.position(7)
        val packed = QrDecoder.luminance(buffer, size, size, rowStride, 2)
        assertArrayEquals(pixels, packed)
        assertEquals(7, buffer.position())
        assertEquals(code, QrDecoder().decode(packed!!, size, size))
    }

    @Test fun rectangularLowContrastCameraFrameDecodesInLandscapeAndPortrait() {
        val matrix = QRCodeWriter().encode(code, BarcodeFormat.QR_CODE, 960, 640)
        val pixels = ByteArray(960 * 640) { if (matrix[it % 960, it / 960]) 45 else 180.toByte() }
        val decoder = QrDecoder()
        assertEquals(code, decoder.decode(pixels, 960, 640))
        val rotated = ByteArray(pixels.size) { index -> pixels[(639 - index % 640) * 960 + index / 640] }
        assertEquals(code, decoder.decode(rotated, 640, 960))
    }

    @Test fun malformedAndOversizedFramesAreRejectedWithoutReadingPastPlane() {
        val decoder = QrDecoder()
        assertNull(decoder.decode(byteArrayOf(), 0, 0))
        assertNull(decoder.decode(byteArrayOf(0), Int.MAX_VALUE, Int.MAX_VALUE))
        assertNull(decoder.decode(ByteArray(8), 3, 3))
        assertNull(QrDecoder.luminance(ByteBuffer.allocate(4), 3, 3, 3, 1))
        assertNull(QrDecoder.luminance(ByteBuffer.allocate(64), 3, 3, 3, 2))
        assertNull(QrDecoder.luminance(ByteBuffer.allocate(64), 3, 3, 3, 0))
        val limited = ByteBuffer.allocate(64).apply { limit(8) }
        assertNull(QrDecoder.luminance(limited, 3, 3, 3, 1))
    }
}
