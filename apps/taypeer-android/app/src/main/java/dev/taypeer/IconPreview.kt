package dev.taypeer

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

/** Only native-validated bounded PNG thumbnails reach the Android bitmap decoder. */
@Composable
internal fun IconPreviewContent(value: RenderedIcon?, fallback: Int, modifier: Modifier = Modifier) {
    val bitmap = remember(value?.identity, value?.png) { value?.let { BitmapFactory.decodeByteArray(it.png, 0, it.png.size)?.asImageBitmap() } }
    if (bitmap != null) Image(bitmap, contentDescription = null, modifier.size(24.dp).testTag("rendered-icon"))
    else Glyph(fallback, modifier)
}

@Composable
internal fun DocumentIcon(state: DocumentState, scope: IconScope, identity: Any?, fallback: Int, modifier: Modifier = Modifier) {
    val binding = state.iconPreviewEpoch
    var visible by remember(binding, scope) { mutableStateOf(false) }
    val value = state.renderedIcon(scope)
    LaunchedEffect(binding, scope, identity, visible, value?.identity) {
        if (visible && value == null) state.loadIconPreview(scope)
    }
    Box(modifier.onGloballyPositioned { coordinates ->
        val bounds = coordinates.boundsInWindow()
        val viewport = coordinates.findRootCoordinates().boundsInWindow()
        visible = bounds.width > 0f && bounds.height > 0f && bounds.overlaps(viewport)
    }) { IconPreviewContent(if (visible) value else null, fallback) }
}
