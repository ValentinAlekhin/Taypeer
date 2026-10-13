package dev.taypeer

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics

@Composable
internal fun SettingsContent(theme: ThemeMode, language: UiLanguage, selectTheme: (ThemeMode) -> Unit, selectLanguage: (UiLanguage) -> Unit,
    databaseOpen: Boolean, metadata: () -> Unit, protection: () -> Unit, generator: () -> Unit, relay: (() -> Unit)? = null,
    devicePreferences: @Composable (() -> Unit)? = null, databaseName: String? = null, writable: Boolean = true, managing: Boolean = true,
    exchange: (() -> Unit)? = null, passwordChange: (() -> Unit)? = null,
    relayPreference: dev.taypeer.bridge.RelayPreference? = null, relayBusy: Boolean = false, relayFailed: Boolean = false,
    selectRelay: (dev.taypeer.bridge.RelayPreference) -> Unit = {}, reloadRelay: () -> Unit = {},
    deviceName: @Composable (() -> Unit)? = null, recovery: (() -> Unit)? = null, saveCopy: (() -> Unit)? = null,
    biometry: @Composable (() -> Unit)? = null, limits: (() -> Unit)? = null) {
    var database by remember { mutableStateOf(false) }
    LaunchedEffect(databaseOpen) { if (!databaseOpen) database = false }
    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        SelectorField(null, stringResource(if (database) R.string.settings_scope_database else R.string.settings_scope_device), "settings-scope") { dismiss ->
            DropdownMenuItem({ Text(stringResource(R.string.settings_scope_device)) }, { dismiss(); database = false }, Modifier.testTag("settings-scope-device"))
            DropdownMenuItem({ Text(stringResource(R.string.settings_scope_database)) }, { dismiss(); database = true }, Modifier.testTag("settings-scope-database"), enabled = databaseOpen)
        }
        if (database) {
            databaseName?.let { Text(it, style = MaterialTheme.typography.titleMedium) }
            FlatAction(R.string.database_info, Modifier.fillMaxWidth().testTag("database-information"), enabled = writable, action = metadata)
            passwordChange?.let { FlatAction(R.string.change_password, Modifier.fillMaxWidth().testTag("settings-change-password"), enabled = managing, action = it) }
            FlatAction(R.string.protection_title, Modifier.fillMaxWidth().testTag("settings-protection"), enabled = databaseOpen, action = protection)
            limits?.let { FlatAction(R.string.limits_title, Modifier.fillMaxWidth().testTag("settings-limits"), enabled = databaseOpen, action = it) }
            exchange?.let { FlatAction(R.string.devices_exchange, Modifier.fillMaxWidth().testTag("settings-exchange"), action = it) }
            recovery?.let { FlatAction(R.string.recover_management, Modifier.fillMaxWidth().testTag("settings-recovery"), action = it) }
            saveCopy?.let { FlatAction(R.string.save_database_copy, Modifier.fillMaxWidth().testTag("settings-save-copy"), action = it) }
        } else {
            deviceName?.invoke()
            LanguageChoices(language, selectLanguage)
            ThemeChoices(theme, selectTheme)
            devicePreferences?.invoke()
            if (databaseOpen) biometry?.invoke()
            relay?.let { RelayPreferenceFields(relayPreference, relayBusy, relayFailed, selectRelay, it, reloadRelay) }
            FlatAction(R.string.generator, Modifier.fillMaxWidth().testTag("settings-generator"), action = generator)
        }
    }
}

/** The closed selector uses the same 48 dp field chrome as text fields. */
@Composable
internal fun SelectorField(label: Int?, value: String, tag: String, enabled: Boolean = true,
    options: @Composable ColumnScope.(dismiss: () -> Unit) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    Box(Modifier.fillMaxWidth()) {
        ActionField(value, label, tag, enabled) { expanded = true }
        DropdownMenu(expanded && enabled, onDismissRequest = { expanded = false }) { options { expanded = false } }
    }
}

@Composable
internal fun LanguageChoices(selected: UiLanguage, select: (UiLanguage) -> Unit) {
    fun label(language: UiLanguage) = if (language == UiLanguage.ENGLISH) "English" else "Русский"
    SelectorField(R.string.language, label(selected), "language-selector") { dismiss ->
        UiLanguage.entries.forEach { language ->
            DropdownMenuItem(text = { Text(label(language)) }, onClick = { dismiss(); select(language) },
                modifier = Modifier.testTag("language-${language.tag}").semantics { this.selected = language == selected },
                trailingIcon = { if (language == selected) Glyph(R.drawable.ic_check) })
        }
    }
}

@Composable
internal fun ThemeChoices(selected: ThemeMode, select: (ThemeMode) -> Unit) {
    val labels = mapOf(ThemeMode.SYSTEM to R.string.theme_system, ThemeMode.LIGHT to R.string.theme_light, ThemeMode.DARK to R.string.theme_dark)
    SelectorField(R.string.theme, stringResource(labels.getValue(selected)), "theme-selector") { dismiss ->
        ThemeMode.entries.forEach { mode ->
            DropdownMenuItem(text = { Text(stringResource(labels.getValue(mode))) }, onClick = { dismiss(); select(mode) },
                modifier = Modifier.testTag("theme-${mode.name.lowercase()}").semantics { this.selected = mode == selected },
                trailingIcon = { if (mode == selected) Glyph(R.drawable.ic_check) })
        }
    }
}
