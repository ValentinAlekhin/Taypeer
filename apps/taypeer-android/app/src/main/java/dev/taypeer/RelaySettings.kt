package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.RelayPreference
import kotlinx.coroutines.*

/** Presentation only; Rust validates and durably changes the device's transport preference. */
internal class RelaySettingsState(private val host: Deferred<Host>, private val scope: CoroutineScope) {
    var preference by mutableStateOf<RelayPreference?>(null); private set
    var busy by mutableStateOf(false); private set
    var failed by mutableStateOf(false); private set

    fun load() {
        if (busy) return
        busy = true; failed = false
        scope.launch {
            try { preference = withContext(Dispatchers.IO) { host.await().relayPreference() } }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { failed = true }
            finally { busy = false }
        }
    }

    fun apply(value: RelayPreference) {
        if (busy) return
        busy = true; failed = false
        scope.launch {
            try {
                val saved = withContext(Dispatchers.IO) {
                    val owner = host.await()
                    owner.setRelayPreference(value)
                    owner.relayPreference()
                }
                preference = saved
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { failed = true }
            finally { busy = false }
        }
    }
}

@Composable
internal fun RelaySettingsScreen(onClose: () -> Unit) {
    val application = LocalContext.current.applicationContext as TaypeerApplication
    val scope = rememberCoroutineScope()
    val state = remember(application, scope) { RelaySettingsState(application.host, scope) }
    LaunchedEffect(state) { state.load() }
    RelaySettingsContent(state.preference, state.busy, state.failed, state::apply, state::load, onClose)
}

@Composable
internal fun RelaySettingsContent(preference: RelayPreference?, busy: Boolean, failed: Boolean,
    apply: (RelayPreference) -> Unit, reload: () -> Unit, close: () -> Unit, startCustom: Boolean = false) {
    var selected by remember(preference) { mutableStateOf(preference) }
    var openedCustom by remember { mutableStateOf(false) }
    var customUrl by remember(preference) { mutableStateOf((preference as? RelayPreference.Custom)?.url.orEmpty()) }
    LaunchedEffect(preference, startCustom) {
        if (startCustom && !openedCustom && preference != null) {
            selected = RelayPreference.Custom(customUrl); openedCustom = true
        }
    }
    BackHandler { if (!busy) close() }
    Scaffold(topBar = { PhoneBar(stringResource(R.string.relay_settings), back = { if (!busy) close() }) },
        bottomBar = { FormActions(close, { selected?.let(apply) }, enabled = preference != null && selected != null && !busy, tag = "apply-relay", label = R.string.apply, cancelEnabled = !busy) }) { insets ->
        Column(Modifier.fillMaxSize().padding(insets).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("relay-loading"))
            Text(stringResource(R.string.relay_explanation))
            RelayChoice(R.string.relay_public, "relay-public", selected == RelayPreference.Public, preference != null && !busy) { selected = RelayPreference.Public }
            RelayChoice(R.string.relay_disabled, "relay-disabled", selected == RelayPreference.Disabled, preference != null && !busy) { selected = RelayPreference.Disabled }
            RelayChoice(R.string.relay_custom, "relay-custom", selected is RelayPreference.Custom, preference != null && !busy) { selected = RelayPreference.Custom(customUrl) }
            if (selected is RelayPreference.Custom) PlainField(customUrl, R.string.relay_address, "relay-url", enabled = !busy, singleLine = false, minLines = 2, maxLines = 4) {
                customUrl = it; selected = RelayPreference.Custom(it)
            }
            Text(stringResource(R.string.relay_direct_warning), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (failed) {
                Text(stringResource(if (preference == null) R.string.relay_load_failed else R.string.relay_change_failed), Modifier.testTag("relay-error"), color = MaterialTheme.colorScheme.error)
                if (preference == null) FlatAction(R.string.retry, Modifier.fillMaxWidth(), enabled = !busy, action = reload)
            }
        }
    }
}

/** The closed field projects only the last acknowledged native preference. */
@Composable
internal fun RelayPreferenceFields(preference: RelayPreference?, busy: Boolean, failed: Boolean,
    apply: (RelayPreference) -> Unit, custom: () -> Unit, reload: () -> Unit) {
    val label = when (preference) {
        RelayPreference.Public -> R.string.relay_public
        RelayPreference.Disabled -> R.string.relay_disabled
        is RelayPreference.Custom -> R.string.relay_custom
        null -> if (failed) R.string.relay_load_failed else R.string.settings_loading
    }
    SelectorField(R.string.relay_settings, stringResource(label), "settings-relay", enabled = preference != null && !busy) { dismiss ->
        DropdownMenuItem({ Text(stringResource(R.string.relay_public)) }, { dismiss(); apply(RelayPreference.Public) }, Modifier.testTag("settings-relay-public"))
        DropdownMenuItem({ Text(stringResource(R.string.relay_disabled)) }, { dismiss(); apply(RelayPreference.Disabled) }, Modifier.testTag("settings-relay-disabled"))
        DropdownMenuItem({ Text(stringResource(R.string.relay_custom)) }, { dismiss(); custom() }, Modifier.testTag("settings-relay-custom"))
    }
    preference?.let { FlatAction(R.string.relay_custom, Modifier.fillMaxWidth().testTag("settings-custom-relay"), enabled = !busy, action = custom) }
    if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("settings-relay-loading"))
    if (failed) {
        Text(stringResource(if (preference == null) R.string.relay_load_failed else R.string.relay_change_failed),
            Modifier.testTag("settings-relay-error"), color = MaterialTheme.colorScheme.error)
        if (preference == null) FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("settings-relay-retry"), enabled = !busy, action = reload)
    }
}

@Composable
private fun RelayChoice(label: Int, tag: String, selected: Boolean, enabled: Boolean, choose: () -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(tag).selectable(selected, enabled, Role.RadioButton, choose), verticalAlignment = Alignment.CenterVertically) {
        RadioButton(selected, onClick = null, enabled = enabled)
        Text(stringResource(label), Modifier.padding(start = 8.dp))
    }
}
