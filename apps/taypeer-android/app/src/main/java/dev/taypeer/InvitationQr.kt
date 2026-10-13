package dev.taypeer

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.WriterException
import com.google.zxing.qrcode.QRCodeWriter

/** Encodes the exact displayed bearer code, including a four-module quiet zone. */
@Composable
fun InvitationQr(code: String, modifier: Modifier = Modifier) {
    val matrix = remember(code) {
        try { QRCodeWriter().encode(code, BarcodeFormat.QR_CODE, 0, 0, mapOf(EncodeHintType.CHARACTER_SET to "UTF-8", EncodeHintType.MARGIN to 4)) }
        catch (_: WriterException) { null }
        catch (_: IllegalArgumentException) { null }
    }
    val description = stringResource(R.string.invitation_qr_description)
    if (matrix == null) Text(stringResource(R.string.invitation_qr_unavailable), color = MaterialTheme.colorScheme.error)
    else Canvas(modifier.aspectRatio(1f).semantics { contentDescription = description }) {
        drawRect(Color.White)
        // Integer module sizes keep edges sharp; the code remains readable in both themes.
        val scale = kotlin.math.floor(size.minDimension / matrix.width).coerceAtLeast(1f)
        val origin = Offset((size.width - matrix.width * scale) / 2, (size.height - matrix.height * scale) / 2)
        for (y in 0 until matrix.height) for (x in 0 until matrix.width) if (matrix[x, y]) {
            drawRect(Color.Black, origin + Offset(x * scale, y * scale), Size(scale, scale))
        }
    }
}
