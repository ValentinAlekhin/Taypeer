package dev.taypeer.probe

import android.content.res.Configuration
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.fragment.app.FragmentActivity
import dev.taypeer.probe.bridge.themePalette
import java.util.Locale

class MainActivity : FragmentActivity() {
    internal val probe: ProbeController by viewModels()
    private lateinit var adapters: PlatformProbes

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        adapters = PlatformProbes(this) { probe.message = it }
        setContent { ProbeScreen(probe, adapters) }
    }

    override fun onPause() {
        // Conservative policy: includes leaving for a system dialog.
        probe.pause()
        super.onPause()
    }

    override fun onResume() {
        super.onResume()
        probe.resume()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && ::adapters.isInitialized) adapters.retryClipboardClear()
    }
}

@Composable
private fun ProbeScreen(probe: ProbeController, adapters: PlatformProbes) {
    val context = LocalContext.current
    val preferences = remember { context.getSharedPreferences("ui-preferences", 0) }
    var language by remember { mutableStateOf(preferences.getString("language", "en")!!) }
    var theme by remember { mutableStateOf(preferences.getString("theme", "system")!!) }
    var fontSize by remember { mutableIntStateOf(preferences.getInt("font-size", 16)) }
    var showInput by remember { mutableStateOf(false) }
    LaunchedEffect(probe.status.generation) { showInput = false }
    val configuration = LocalConfiguration.current
    val localized = remember(context, language, configuration) {
        context.createConfigurationContext(Configuration(configuration).apply {
            setLocale(Locale.forLanguageTag(language))
        })
    }
    val dark = when (theme) {
        "light" -> false
        "dark" -> true
        else -> isSystemInDarkTheme()
    }
    val palette = remember(dark) { themePalette(dark) }
    fun color(role: String) = Color(0xff000000L or palette.getValue(role).toLong())
    val baseColors = if (dark) darkColorScheme() else lightColorScheme()
    val colors = baseColors.copy(
        background = color("background"), onBackground = color("foreground"),
        surface = color("panel"), onSurface = color("foreground"),
        surfaceVariant = color("background"), onSurfaceVariant = color("muted"),
        outline = color("border"), outlineVariant = color("border"),
        primary = color("primary"), onPrimary = color("on_primary"),
        secondaryContainer = color("selection"), onSecondaryContainer = color("foreground"),
        error = color("danger"), surfaceTint = Color.Transparent)
    MaterialTheme(colorScheme = colors) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.safeDrawingPadding().imePadding().padding(12.dp)) {
                Text(localized.getString(R.string.app_name), style = MaterialTheme.typography.titleLarge)
                Text(localized.getString(R.string.synthetic_only), fontSize = 12.sp)
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    TextButton(onClick = {
                        language = if (language == "en") "ru" else "en"
                        preferences.edit().putString("language", language).apply()
                    }, modifier = Modifier.testTag("language")) { Text("EN / RU") }
                    listOf("system" to R.string.theme_system, "light" to R.string.theme_light,
                        "dark" to R.string.theme_dark).forEach { (value, label) ->
                        TextButton(onClick = {
                            theme = value
                            preferences.edit().putString("theme", value).apply()
                        }, modifier = Modifier.testTag("theme-$value")) {
                            Text(localized.getString(label))
                        }
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = probe::openSample, modifier = Modifier.testTag("open"),
                        enabled = probe.input.text.isNotEmpty()) {
                        Text(localized.getString(R.string.open_sample))
                    }
                    OutlinedButton(onClick = probe::lockNow, modifier = Modifier.testTag("lock")) {
                        Text(localized.getString(R.string.lock))
                    }
                }
                Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                    Text(if (probe.status.locked) localized.getString(R.string.locked)
                        else localized.getString(R.string.opened, probe.status.characters.toInt()),
                        modifier = Modifier.testTag("status"), fontSize = fontSize.sp)
                    val inputGeneration = probe.status.generation
                    key(inputGeneration) {
                    OutlinedTextField(value = probe.input, onValueChange = { probe.edit(it, inputGeneration) },
                        label = { Text(localized.getString(R.string.sample)) },
                        singleLine = true, modifier = Modifier.fillMaxWidth().testTag("sample"),
                        textStyle = LocalTextStyle.current.copy(fontSize = fontSize.sp),
                        keyboardOptions = KeyboardOptions(autoCorrectEnabled = false,
                            keyboardType = KeyboardType.Password),
                        visualTransformation = if (showInput) VisualTransformation.None
                            else PasswordVisualTransformation(),
                        trailingIcon = { TextButton(onClick = { showInput = !showInput }) {
                            Text(localized.getString(if (showInput) R.string.hide else R.string.reveal))
                        } })
                    }
                    Row {
                        TextButton(onClick = { if (probe.revealed.isEmpty()) probe.reveal() else probe.hide() },
                            enabled = !probe.status.locked, modifier = Modifier.testTag("reveal")) {
                            Text(localized.getString(if (probe.revealed.isEmpty()) R.string.reveal else R.string.hide))
                        }
                        TextButton(onClick = { probe.reveal(delayed = true) },
                            enabled = !probe.status.locked, modifier = Modifier.testTag("delayed")) {
                            Text(localized.getString(R.string.delayed))
                        }
                    }
                    Text(probe.revealed, modifier = Modifier.testTag("revealed"), fontSize = fontSize.sp)
                    probe.message?.let { Text(localized.getString(it), modifier = Modifier.testTag("message")) }
                    TextButton(onClick = {
                        probe.sampleForCopy()?.let { adapters.copy(it) }
                    }, enabled = !probe.status.locked) { Text(localized.getString(R.string.copy)) }
                    Row {
                        TextButton(onClick = adapters::importSample) { Text(localized.getString(R.string.import_sample)) }
                        TextButton(onClick = adapters::exportSample) { Text(localized.getString(R.string.export_sample)) }
                    }
                    TextButton(onClick = { adapters.authenticate(localized) }) { Text(localized.getString(R.string.biometric)) }
                    Row {
                        TextButton(onClick = { probe.dialog = true }, modifier = Modifier.testTag("dialog")) {
                            Text(localized.getString(R.string.dialog))
                        }
                        listOf(14, 16, 18).forEach { size ->
                            TextButton(onClick = {
                                fontSize = size
                                preferences.edit().putInt("font-size", size).apply()
                            }) { Text(size.toString()) }
                        }
                    }
                    repeat(100) { index ->
                        Text(localized.getString(R.string.entry, index + 1),
                            Modifier.fillMaxWidth().padding(vertical = 10.dp), fontSize = fontSize.sp)
                        HorizontalDivider()
                    }
                }
                if (probe.dialog) AlertDialog(onDismissRequest = { probe.dialog = false },
                    title = { Text(localized.getString(R.string.dialog)) },
                    text = { Text(localized.getString(R.string.synthetic_only)) },
                    confirmButton = { TextButton(onClick = { probe.dialog = false }) {
                        Text(localized.getString(R.string.close))
                    } })
            }
        }
    }
}
