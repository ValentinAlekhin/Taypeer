package dev.taypeer

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import dev.taypeer.bridge.IconScope
import dev.taypeer.bridge.RenderedIcon
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Composition owns only visible bounded thumbnails, always read from the result's exact worker. */
@Composable
internal fun SearchIcon(state: SearchState, row: SearchResultRow, fallback: Int) {
    val epoch = state.previewEpoch
    var visible by remember(row.sourceKey, row.entry.id) { mutableStateOf(false) }
    var value by remember(row.sourceKey, row.entry.id, row.entry.icon, epoch) { mutableStateOf<RenderedIcon?>(null) }
    DisposableEffect(row.sourceKey, row.entry.id, row.entry.icon, epoch) {
        onDispose { value?.png?.fill(0) }
    }
    LaunchedEffect(row.sourceKey, row.entry.id, row.entry.icon, epoch, visible) {
        if (!visible) { value?.png?.fill(0); value = null; return@LaunchedEffect }
        val source = state.source(row) ?: return@LaunchedEffect
        var reply: RenderedIcon? = null
        try { withContext(Dispatchers.IO) { reply = source.session.iconPreview(IconScope.Entry(row.entry.id)) } }
        catch (cancelled: CancellationException) { reply?.png?.fill(0); throw cancelled }
        catch (_: Exception) { reply?.png?.fill(0); reply = null }
        if (!source.permitted() || state.source(row) !== source || state.previewEpoch != epoch) {
            reply?.png?.fill(0); return@LaunchedEffect
        }
        value?.png?.fill(0); value = reply
    }
    Box(Modifier.onGloballyPositioned { coordinates ->
        val bounds = coordinates.boundsInWindow()
        val viewport = coordinates.findRootCoordinates().boundsInWindow()
        visible = bounds.width > 0f && bounds.height > 0f && bounds.overlaps(viewport)
    }) { IconPreviewContent(if (visible) value else null, fallback) }
}
