package dev.taypeer

import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.ui.graphics.Color
import dev.taypeer.bridge.palette

@Composable
fun TaypeerTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val colors = remember(dark) {
        val p = palette(dark)
        fun rgb(value: UInt) = Color(0xff000000L or value.toLong())
        val defaults = if (dark) darkColorScheme() else lightColorScheme()
        defaults.copy(background = rgb(p.background), onBackground = rgb(p.foreground),
            surface = rgb(p.panel), onSurface = rgb(p.foreground), onSurfaceVariant = rgb(p.muted),
            primary = rgb(p.primary), onPrimary = rgb(p.onPrimary), outline = rgb(p.border),
            error = rgb(p.danger), surfaceVariant = rgb(p.chrome))
    }
    MaterialTheme(colorScheme = colors, content = content)
}
