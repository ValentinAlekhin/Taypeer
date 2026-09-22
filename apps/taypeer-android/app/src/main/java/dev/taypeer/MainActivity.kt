package dev.taypeer

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier

class MainActivity : ComponentActivity() {
    private val app get() = application as TaypeerApplication
    private val generator by viewModels<GeneratorState>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        setContent {
            TaypeerTheme {
                Surface(Modifier.fillMaxSize()) {
                    GeneratorScreen(generator) { value ->
                        try { app.clipboard.copy(value); generator.error = false }
                        catch (_: Exception) { generator.error = true }
                    }
                }
            }
        }
    }
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        try { app.clipboard.focusChanged(hasFocus) } catch (_: Exception) { generator.error = true }
    }
    override fun onStop() {
        if (!isChangingConfigurations) generator.lock()
        super.onStop()
    }
}
