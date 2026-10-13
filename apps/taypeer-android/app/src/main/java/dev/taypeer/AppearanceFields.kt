package dev.taypeer

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.taypeer.bridge.bundledIconKeys
import dev.taypeer.bridge.AppearanceView
import dev.taypeer.bridge.IconView
import kotlin.math.roundToInt

private val bundledAssets = mapOf(
    "bookmark" to R.drawable.ic_bookmark,
    "credit-card" to R.drawable.ic_credit_card,
    "database" to R.drawable.ic_database,
    "download" to R.drawable.ic_download,
    "file" to R.drawable.ic_file,
    "file-key-2" to R.drawable.ic_file_key_2,
    "fingerprint" to R.drawable.ic_fingerprint,
    "folder" to R.drawable.ic_folder,
    "folder-open" to R.drawable.ic_folder_open,
    "globe" to R.drawable.ic_globe,
    "heart" to R.drawable.ic_heart,
    "home" to R.drawable.ic_home,
    "key-round" to R.drawable.ic_key_round,
    "laptop" to R.drawable.ic_laptop,
    "lock" to R.drawable.ic_lock,
    "lock-keyhole" to R.drawable.ic_lock_keyhole,
    "mail" to R.drawable.ic_mail,
    "refresh-cw" to R.drawable.ic_refresh_cw,
    "server" to R.drawable.ic_server,
    "shield" to R.drawable.ic_shield,
    "smartphone" to R.drawable.ic_smartphone,
    "star" to R.drawable.ic_star,
    "terminal" to R.drawable.ic_terminal,
    "trash-2" to R.drawable.ic_trash_2,
    "user" to R.drawable.ic_user,
    "users" to R.drawable.ic_users,
    "wallet" to R.drawable.ic_wallet,
)

internal fun bundledIconDrawable(key: String?): Int = bundledAssets[key] ?: R.drawable.ic_key_round

internal fun savedIconDrawable(value: IconView?, default: Int): Int = when (value) {
    is IconView.Lucide -> bundledAssets[value.key] ?: default
    IconView.Image -> R.drawable.ic_image
    else -> default
}

internal fun appearanceIconDrawable(value: AppearanceView?): Int =
    if (value?.image != null) R.drawable.ic_image else bundledAssets[value?.lucide] ?: R.drawable.ic_key_round

@Composable
internal fun AppearanceEditor(value: AppearanceView?, icon: (String?) -> Unit,
    foreground: (UInt?) -> Unit, background: (UInt?) -> Unit, reset: () -> Unit, title: String = "", enabled: Boolean = true, actions: IconPickerActions? = null, previewContent: (@Composable () -> Unit)? = null) {
    Text(stringResource(R.string.appearance), style = MaterialTheme.typography.titleMedium)
    NavigationRow(title, appearanceIconDrawable(value), subtitle = stringResource(R.string.entry_preview), leading = previewContent, action = null)
    IconPicker(value?.lucide, icon, showPreview = false, enabled = enabled, actions = actions)
    ColorPicker(R.string.foreground_color, value?.foreground, foreground, enabled)
    ColorPicker(R.string.background_color, value?.background, background, enabled)
    FlatAction(R.string.reset_appearance, Modifier.fillMaxWidth(), enabled = enabled, action = reset)
}

@Composable
internal fun IconPicker(current: String?, choose: (String?) -> Unit, preview: Int? = null, showPreview: Boolean = true,
    enabled: Boolean = true, actions: IconPickerActions? = null, previewContent: (@Composable () -> Unit)? = null) {
    var page by remember { mutableStateOf(0) }
    var url by remember { mutableStateOf("") }
    var submitted by remember { mutableStateOf<Long?>(null) }
    val blocked = actions?.attempt != null
    fun close() { if (!blocked) page = 0 }
    LaunchedEffect(actions?.confirmation) {
        submitted?.let { before -> if ((actions?.confirmation ?: 0L) > before) { page = 0; submitted = null } }
    }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        if (showPreview) {
            if (previewContent != null) Box(Modifier.padding(end = 16.dp)) { previewContent() }
            else Glyph(preview ?: bundledIconDrawable(current), Modifier.padding(end = 16.dp))
        }
        FlatAction(R.string.choose_icon, Modifier.weight(1f).testTag("icon-picker"), enabled = enabled && !blocked) {
            actions?.loadCandidate?.invoke(); page = 1
        }
    }
    if (page != 0) Dialog(onDismissRequest = ::close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        if (page == 1) IconSourceSheet(actions?.candidate, { choose(null); page = 0 }, { page = 2 },
            { actions?.candidate?.let { actions.stored(it.identity); submitted = actions.confirmation } },
            { if (actions != null) page = 3 }, { actions?.file?.invoke(); page = 0 }, ::close,
            enabled = !blocked, status = {
                actions?.failure?.let { Text(stringResource(iconFailureString(it)), Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error) }
                if (actions?.attempt?.busy == true) LinearProgressIndicator(Modifier.fillMaxWidth())
                if (actions?.attempt?.uncertain == true) {
                    Text(stringResource(R.string.icon_outcome_unknown), Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error)
                    FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-stored-icon")) { actions.retry() }
                }
            }) else Surface(Modifier.fillMaxSize()) {
            if (page == 2) LucidePickerContent(current, { choose(it); page = 0 }, ::close)
            else IconUrlContent(url, { url = it }, actions?.attempt?.busy == true, actions?.attempt?.uncertain == true,
                actions?.failure, {
                    actions?.let { owner ->
                        submitted = owner.confirmation
                        if (owner.attempt != null) owner.retry() else owner.download(url)
                    }
                }, ::close)
        }
    }
}

private fun rgbaColor(value: UInt): Color = Color((((value and 0xffu) shl 24) or (value shr 8)).toInt())

@Composable
internal fun ColorPicker(label: Int, value: UInt?, choose: (UInt?) -> Unit, enabled: Boolean = true) {
    var open by remember { mutableStateOf(false) }
    ActionField(value?.let { "#${it.toString(16).padStart(8, '0')}" } ?: stringResource(R.string.follow_theme), label, "color-picker-$label", enabled = enabled,
        leading = { if (value != null) { Surface(Modifier.size(24.dp), color = rgbaColor(value)) {}; Spacer(Modifier.width(12.dp)) } }) { open = true }
    if (open) {
        var editing by remember { mutableStateOf(value ?: 0x000000ffu) }
        Dialog(onDismissRequest = { open = false }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
            Surface(Modifier.fillMaxSize()) {
                Scaffold(topBar = { PhoneBar(stringResource(label), back = { open = false }) }, bottomBar = { FormActions({ open = false }, { choose(editing); open = false }, true, label = R.string.apply) }) { padding ->
                    Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        Surface(Modifier.fillMaxWidth().height(120.dp), color = rgbaColor(editing)) {}
                        Text("#${editing.toString(16).padStart(8, '0')}")
                        listOf("R", "G", "B", "A").forEachIndexed { index, channel ->
                            val shift = (3 - index) * 8
                            val component = ((editing shr shift) and 0xffu).toInt()
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(channel)
                                Slider(component.toFloat(), { position ->
                                    val replacement = position.roundToInt().coerceIn(0, 255).toUInt()
                                    editing = (editing and (0xffu shl shift).inv()) or (replacement shl shift)
                                }, valueRange = 0f..255f, steps = 254, modifier = Modifier.weight(1f))
                                Text(component.toString())
                            }
                        }
                        FlatAction(R.string.follow_theme, Modifier.fillMaxWidth()) { choose(null); open = false }
                    }
                }
            }
        }
    }
}
