package dev.taypeer

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

internal data class IconPickerActions(
    val candidate: StoredFaviconView?, val loadCandidate: () -> Unit,
    val stored: (String) -> Unit, val download: (String) -> Unit, val file: () -> Unit,
    val attempt: IconAttempt?, val failure: IconFailure?, val confirmation: Long, val retry: () -> Unit,
)

@Composable
internal fun IconSourceSheet(candidate: StoredFaviconView?, default: () -> Unit, lucide: () -> Unit,
    stored: () -> Unit, url: () -> Unit, file: () -> Unit, close: () -> Unit, enabled: Boolean = true,
    status: @Composable () -> Unit = {}) {
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.55f)).clickable(onClick = close)) {
        Surface(Modifier.align(Alignment.BottomCenter).fillMaxWidth().clickable { },
            shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp)) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Box(Modifier.fillMaxWidth().height(20.dp)) {
                    Box(Modifier.align(Alignment.TopCenter).padding(top = 8.dp).size(width = 40.dp, height = 4.dp)
                        .background(MaterialTheme.colorScheme.onSurfaceVariant, RoundedCornerShape(2.dp)))
                }
                IconSourceContent(candidate, default, lucide, stored, url, file, enabled)
                status()
            }
        }
    }
}

@Composable
internal fun IconSourceContent(candidate: StoredFaviconView?, default: () -> Unit, lucide: () -> Unit,
    stored: () -> Unit, url: () -> Unit, file: () -> Unit, enabled: Boolean = true) {
    Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 12.dp)) {
        Text(stringResource(R.string.icon_title), Modifier.padding(start = 8.dp, top = 8.dp, bottom = 12.dp).heightIn(min = 28.dp), style = MaterialTheme.typography.titleMedium)
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            IconSourceRow(R.string.default_icon, "icon-source-default", enabled, default)
            IconSourceRow(R.string.lucide_icons, "icon-source-lucide", enabled, lucide)
            IconSourceRow(R.string.loaded_favicon, "icon-source-stored", enabled && candidate?.available == true, stored,
                if (candidate != null && !candidate.available) R.string.icon_original_unavailable else null)
            IconSourceRow(R.string.icon_from_url, "icon-source-url", enabled, url)
            IconSourceRow(R.string.icon_from_file, "icon-source-file", enabled, file)
        }
    }
}

@Composable
private fun IconSourceRow(label: Int, tag: String, enabled: Boolean, action: () -> Unit, caption: Int? = null) {
    Column(Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(tag)
        .clickable(enabled = enabled, onClick = action).padding(horizontal = 8.dp, vertical = 12.dp)) {
        Text(stringResource(label), color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)
        caption?.let { Text(stringResource(it), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}

@Composable
internal fun LucidePickerContent(current: String?, choose: (String?) -> Unit, close: () -> Unit) {
    var query by remember { mutableStateOf("") }
    var selected by remember(current) { mutableStateOf(current) }
    val keys = remember { bundledIconKeys() }
    Scaffold(topBar = { PhoneBar(stringResource(R.string.lucide_picker_title), back = close) },
        bottomBar = { FormActions(close, { choose(selected) }, true, tag = "choose-lucide", label = R.string.select_icon) }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SearchInput(query, { query = it }, R.string.search_icons, "lucide-search")
            val filtered = keys.filter { it.contains(query, ignoreCase = true) }
            Column(Modifier.horizontalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                filtered.chunked(5).forEach { row ->
                    Row(Modifier.width(328.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        row.forEach { key ->
                            Box(Modifier.size(56.dp).testTag("icon-key-$key")
                                .semantics { contentDescription = key }
                                .background(if (selected == key) LocalSelectionColor.current else Color.Transparent, RoundedCornerShape(6.dp))
                                .clickable { selected = key }, contentAlignment = Alignment.Center) {
                                Glyph(bundledIconDrawable(key))
                            }
                        }
                        repeat(5 - row.size) { Spacer(Modifier.width(56.dp)) }
                    }
                }
            }
            if (filtered.isEmpty()) Text(stringResource(R.string.no_search_results))
        }
    }
}

@Composable
internal fun IconUrlContent(value: String, update: (String) -> Unit, busy: Boolean, uncertain: Boolean,
    failure: IconFailure?, download: () -> Unit, close: () -> Unit) {
    Scaffold(topBar = { PhoneBar(stringResource(R.string.download_icon), back = { if (!busy && !uncertain) close() }) },
        bottomBar = { FormActions(close, download, enabled = !busy && value.isNotEmpty(), tag = "download-icon",
            label = if (uncertain) R.string.retry else R.string.download_icon_action, cancelEnabled = !busy && !uncertain) }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            PlainField(value, R.string.image_url, "icon-image-url", enabled = !busy && !uncertain, singleLine = false,
                inputModifier = Modifier.heightIn(min = 64.dp), update = update)
            Text(stringResource(R.string.icon_download_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("icon-loading"))
            if (uncertain) Text(stringResource(R.string.icon_outcome_unknown), Modifier.testTag("icon-unknown"), color = MaterialTheme.colorScheme.error)
            failure?.let { Text(stringResource(iconFailureString(it)), Modifier.testTag("icon-acquisition-error"), color = MaterialTheme.colorScheme.error) }
        }
    }
}

internal fun iconFailureString(value: IconFailure): Int = when (value) {
    IconFailure.INVALID_URL -> R.string.icon_invalid_url
    IconFailure.NETWORK_BOUNDARY -> R.string.icon_network_boundary
    IconFailure.NETWORK -> R.string.icon_network_failed
    IconFailure.TIMEOUT -> R.string.icon_timeout
    IconFailure.TOO_LARGE -> R.string.icon_too_large
    IconFailure.INVALID_IMAGE -> R.string.icon_invalid_image
    IconFailure.FILE -> R.string.icon_file_failed
}
