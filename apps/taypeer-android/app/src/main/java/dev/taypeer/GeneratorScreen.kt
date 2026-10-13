package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.GeneratedQuality
import java.text.NumberFormat
import kotlin.math.roundToInt

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GeneratorScreen(state: GeneratorState, back: () -> Unit = {}, use: (() -> Unit)? = null, copy: (String) -> Unit) {
    var exclusions by remember { mutableStateOf(false) }
    var modes by remember { mutableStateOf(false) }
    LaunchedEffect(state) { if (state.generated.isEmpty() && !state.running && !state.error) state.generate() }
    if (exclusions) {
        GeneratorExclusions(state) { exclusions = false }
        return
    }
    BackHandler(onBack = back)
    Scaffold(topBar = { PhoneBar("", back = back) }, bottomBar = {
        if (use != null) FormActions(back, use, enabled = state.generated.isNotEmpty() && !state.running,
            label = R.string.use_password, tag = "use-generated-password", cancelLabel = R.string.close, cancelEnabled = true)
        else Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer)) {
            HorizontalDivider()
            FlatAction(R.string.close, Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("close-generator"), action = back)
        }
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding().padding(16.dp)
            .verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            PlainField(state.generated, R.string.generated_value,
                if (state.generated.isEmpty()) "generator-empty" else "generated-password", enabled = false,
                transformation = if (state.revealed) VisualTransformation.None else PasswordVisualTransformation(),
                actions = {
                    IconAction(if (state.revealed) R.drawable.ic_eye_off else R.drawable.ic_eye,
                        if (state.revealed) R.string.hide else R.string.reveal, Modifier.testTag("generator-reveal"),
                        enabled = state.generated.isNotEmpty()) { state.revealed = !state.revealed }
                    IconAction(R.drawable.ic_dice_5, R.string.generate, Modifier.testTag("generate-password"), enabled = !state.running, action = state::generate)
                    IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("generator-copy"),
                        enabled = state.generated.isNotEmpty() && !state.running) { copy(state.generated) }
                }, update = {})
            if (state.generated.isNotEmpty()) GeneratorQuality(state)
            if (state.running) LinearProgressIndicator(Modifier.fillMaxWidth().height(4.dp))
            if (state.error) Text(stringResource(R.string.operation_failed), color = MaterialTheme.colorScheme.error,
                modifier = Modifier.testTag("generator-error"))
            Box {
                Button({ modes = true }, Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("generator-mode"),
                    shape = androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 12.dp),
                    colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.surfaceVariant,
                        contentColor = MaterialTheme.colorScheme.onSurface)) {
                    Text(stringResource(if (state.passphrase) R.string.passphrase else R.string.password), Modifier.weight(1f))
                    Glyph(R.drawable.ic_chevron_down)
                }
                DropdownMenu(modes, { modes = false }) {
                    DropdownMenuItem({ Text(stringResource(R.string.password)) }, {
                        state.selectMode(GeneratorMode.PASSWORD); modes = false
                    }, Modifier.testTag("generator-password-mode"))
                    DropdownMenuItem({ Text(stringResource(R.string.passphrase)) }, {
                        state.selectMode(GeneratorMode.PASSPHRASE); modes = false
                    }, Modifier.testTag("generator-passphrase-mode"))
                }
            }
            if (state.passphrase) {
                PlainField(state.words, R.string.generator_word_count, "generator-words",
                    keyboard = KeyboardOptions(keyboardType = KeyboardType.Number), update = state::changeWords)
                PlainField(state.separator, R.string.separator, "generator-separator", update = state::changeSeparator)
                Text(stringResource(R.string.generator_wordlist), style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant)
            } else {
                PlainField(state.length, R.string.generator_length, "length",
                    keyboard = KeyboardOptions(keyboardType = KeyboardType.Number), update = state::changeLength)
                Slider(state.length.toFloatOrNull()?.coerceIn(1f, 256f) ?: 1f,
                    { state.changeLength(it.roundToInt().toString()) }, Modifier.fillMaxWidth().testTag("generator-length-slider"),
                    valueRange = 1f..256f, steps = 254,
                    thumb = { Box(Modifier.size(16.dp).background(MaterialTheme.colorScheme.onSurface, CircleShape)) },
                    track = { Box(Modifier.fillMaxWidth().height(2.dp).background(MaterialTheme.colorScheme.outlineVariant)) })
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    GeneratorChoice(R.string.generator_uppercase, state.uppercase, "generator-uppercase", state::changeUppercase)
                    GeneratorChoice(R.string.generator_lowercase, state.lowercase, "generator-lowercase", state::changeLowercase)
                    GeneratorChoice(R.string.generator_digits, state.digits, "generator-digits", state::changeDigits)
                    GeneratorChoice(R.string.generator_punctuation, state.punctuation, "generator-punctuation", state::changePunctuation)
                }
                FlatAction(R.string.generator_exclusions, Modifier.fillMaxWidth().testTag("open-generator-exclusions")) { exclusions = true }
            }
        }
    }
}

@Composable
private fun GeneratorQuality(state: GeneratorState) {
    val color = when (state.quality) {
        GeneratedQuality.LOW -> MaterialTheme.colorScheme.error
        GeneratedQuality.MEDIUM -> LocalWarningColor.current
        GeneratedQuality.HIGH -> LocalSuccessColor.current
    }
    val label = when (state.quality) {
        GeneratedQuality.LOW -> R.string.generator_quality_low
        GeneratedQuality.MEDIUM -> R.string.generator_quality_medium
        GeneratedQuality.HIGH -> R.string.generator_quality_high
    }
    val number = NumberFormat.getNumberInstance(LocalConfiguration.current.locales[0]).apply {
        minimumFractionDigits = 2; maximumFractionDigits = 2
    }.format(state.entropyBits)
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.fillMaxWidth().height(4.dp).background(MaterialTheme.colorScheme.outlineVariant)) {
            Box(Modifier.fillMaxWidth(when (state.quality) {
                GeneratedQuality.LOW -> 1f / 3; GeneratedQuality.MEDIUM -> 2f / 3; GeneratedQuality.HIGH -> 1f
            }).fillMaxHeight().background(color))
        }
        Text(stringResource(if (state.passphrase) R.string.generator_phrase_quality else R.string.generator_password_quality,
            stringResource(label), if (state.passphrase) state.words else state.length, number),
            style = MaterialTheme.typography.bodyMedium, modifier = Modifier.testTag("generator-quality"))
    }
}

@Composable
private fun GeneratorExclusions(state: GeneratorState, back: () -> Unit) {
    var similar by remember { mutableStateOf(state.excludeSimilar) }
    var characters by remember { mutableStateOf(state.exclusions) }
    BackHandler(onBack = back)
    Scaffold(topBar = { PhoneBar(stringResource(R.string.generator_exclusions_title), back = back) }, bottomBar = {
        FormActions(back, { state.changeSimilar(similar); state.changeExclusions(characters); back() },
            enabled = true, label = R.string.apply, tag = "apply-generator-exclusions")
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding().padding(16.dp)
            .verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            GeneratorChoice(R.string.exclude_similar, similar, "generator-similar") { similar = it }
            PlainField(characters, R.string.exclusions, "generator-exclusions", update = { characters = it })
            Text(stringResource(R.string.generator_options_note), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun GeneratorChoice(label: Int, checked: Boolean, tag: String, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).background(if (checked) LocalSelectionColor.current else MaterialTheme.colorScheme.surface, RoundedCornerShape(6.dp))
        .testTag(tag).toggleable(checked, role = Role.Checkbox, onValueChange = update),
        verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
            Checkbox(checked, null, colors = CheckboxDefaults.colors(checkedColor = LocalSelectionColor.current,
                checkmarkColor = MaterialTheme.colorScheme.onSurfaceVariant, uncheckedColor = MaterialTheme.colorScheme.outline))
        }
        Text(stringResource(label), Modifier.weight(1f).padding(end = 12.dp))
    }
}
