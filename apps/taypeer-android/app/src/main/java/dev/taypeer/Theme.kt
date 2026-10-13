package dev.taypeer

import android.content.Context
import android.content.ContextWrapper
import android.content.res.Configuration
import android.content.res.Resources
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.taypeer.bridge.palette

enum class ThemeMode { SYSTEM, LIGHT, DARK }
enum class UiLanguage(val tag: String) { ENGLISH("en"), RUSSIAN("ru") }

/** Device preferences contain no database names, drafts or protected values. */
class UiPreferences(context: Context) {
    private val storage = context.getSharedPreferences("presentation", Context.MODE_PRIVATE)
    var theme by mutableStateOf(runCatching { ThemeMode.valueOf(storage.getString("theme", "SYSTEM")!!) }.getOrDefault(ThemeMode.SYSTEM))
        private set
    var language by mutableStateOf(runCatching { UiLanguage.valueOf(storage.getString("language", "ENGLISH")!!) }.getOrDefault(UiLanguage.ENGLISH))
        private set
    fun selectTheme(value: ThemeMode) {
        storage.edit().putString("theme", value.name).apply()
        theme = value
    }
    fun selectLanguage(value: UiLanguage) {
        storage.edit().putString("language", value.name).apply()
        language = value
    }
}

val LocalUiPreferences = staticCompositionLocalOf<UiPreferences> { error("UI preferences are unavailable") }
val LocalSelectionColor = staticCompositionLocalOf { Color(0xff292929) }
internal val LocalSuccessColor = staticCompositionLocalOf { Color(0xff247744) }
internal val LocalWarningColor = staticCompositionLocalOf { Color(0xff8b6508) }

@Composable
fun TaypeerTheme(mode: ThemeMode? = null, language: UiLanguage? = null, content: @Composable () -> Unit) {
    val base = LocalContext.current
    val configuration = LocalConfiguration.current
    val application = base.applicationContext as TaypeerApplication
    val preferences = application.uiPreferences
    val dark = when (mode ?: preferences.theme) {
        ThemeMode.SYSTEM -> isSystemInDarkTheme()
        ThemeMode.LIGHT -> false
        ThemeMode.DARK -> true
    }
    val p = remember(dark) { palette(dark) }
    val selectedLanguage = language ?: preferences.language
    val localizedConfiguration = remember(configuration, selectedLanguage) {
        Configuration(configuration).apply { setLocale(java.util.Locale.forLanguageTag(selectedLanguage.tag)) }
    }
    val localizedContext = remember(base, localizedConfiguration) {
        val resources = base.createConfigurationContext(localizedConfiguration).resources
        object : ContextWrapper(base) { override fun getResources(): Resources = resources }
    }
    fun rgb(value: UInt) = Color(0xff000000L or value.toLong())
    val colors = remember(dark) {
        val defaults = if (dark) darkColorScheme() else lightColorScheme()
        defaults.copy(
            background = rgb(p.background), onBackground = rgb(p.foreground),
            surface = rgb(p.background), onSurface = rgb(p.foreground), onSurfaceVariant = rgb(p.muted),
            surfaceVariant = rgb(p.chrome), primary = rgb(p.primary), onPrimary = rgb(p.onPrimary),
            secondary = rgb(p.foreground), onSecondary = rgb(p.background),
            secondaryContainer = rgb(p.selection), onSecondaryContainer = rgb(p.foreground),
            outline = rgb(p.border), outlineVariant = rgb(p.border), error = rgb(p.danger),
            surfaceContainer = rgb(p.panel), surfaceContainerHigh = rgb(p.chrome),
            surfaceContainerHighest = rgb(p.chrome), surfaceContainerLow = rgb(p.background),
        )
    }
    val text = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 16.sp, lineHeight = 24.sp)
    CompositionLocalProvider(LocalUiPreferences provides preferences, LocalSelectionColor provides rgb(p.selection),
        LocalSuccessColor provides rgb(p.success), LocalWarningColor provides rgb(p.warning),
        LocalContext provides localizedContext, LocalConfiguration provides localizedConfiguration) {
        MaterialTheme(
            colorScheme = colors,
            typography = Typography(
                bodyLarge = text, bodyMedium = text, bodySmall = text.copy(fontSize = 14.sp, lineHeight = 20.sp),
                titleLarge = text.copy(fontSize = 18.sp, lineHeight = 28.sp, fontWeight = FontWeight.Medium),
                titleMedium = text.copy(fontSize = 18.sp, lineHeight = 28.sp, fontWeight = FontWeight.Medium),
                labelLarge = text.copy(fontWeight = FontWeight.Medium),
                labelMedium = text.copy(fontSize = 14.sp, lineHeight = 20.sp),
            ),
            shapes = Shapes(extraSmall = RoundedCornerShape(4.dp), small = RoundedCornerShape(6.dp), medium = RoundedCornerShape(6.dp)),
            content = content,
        )
    }
}
