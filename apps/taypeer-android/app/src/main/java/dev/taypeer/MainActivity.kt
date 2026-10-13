package dev.taypeer

import android.os.Bundle
import android.view.WindowManager
import android.widget.Toast
import androidx.fragment.app.FragmentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.BackHandler
import androidx.activity.viewModels
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

class MainActivity : FragmentActivity() {
    private val app get() = application as TaypeerApplication
    private val generator by viewModels<GeneratorState>()
    private val documents by viewModels<DocumentState>()
    internal val documentState get() = documents
    internal val generatorState get() = generator
    private var secretMaskEpoch by mutableStateOf(0L)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        app.osBiometry.attach(this)
        app.deviceSettings.load()
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        setContent {
            CompositionLocalProvider(LocalSecretMaskEpoch provides secretMaskEpoch) { TaypeerTheme {
                val attachments = rememberAttachmentActions(documents)
                Surface(Modifier.fillMaxSize()) {
                    if (documents.generator) {
                        GeneratorScreen(generator, back = { generator.lock(); documents.generator = false }, use = if (documents.entry != null) ({
                            documents.edit(dev.taypeer.bridge.EntryTextField.PASSWORD, generator.generated)
                            generator.lock(); documents.generator = false
                        }) else null) { value ->
                            try { copySensitive(value); generator.error = false }
                            catch (_: Exception) { generator.error = true }
                        }
                    } else LibraryScreen(documents, attachments, copySecret = ::copySecret) { value -> copySecret(value) }
                }
            } }
        }
    }
    private fun copySensitive(value: String) {
        // Policy I/O completes before copying is offered. Never defer a captured
        // secret across a settings load, lock or Activity lifecycle transition.
        val settings = app.deviceSettings.settings ?: throw dev.taypeer.bridge.AndroidException.Runtime()
        app.clipboard.copy(value, settings.clipboardSeconds)
    }
    private fun copySecret(value: String): Boolean = try {
        copySensitive(value)
        Toast.makeText(this, getString(R.string.clipboard_copied), Toast.LENGTH_SHORT).show()
        true
    } catch (_: Exception) {
        documents.reportFailure()
        false
    }
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        try { app.clipboard.focusChanged(hasFocus) } catch (_: Exception) { generator.error = true }
        if (!hasFocus) { secretMaskEpoch++; documents.hideSecrets(); generator.hideSecret() }
    }
    override fun onUserInteraction() {
        super.onUserInteraction()
        documents.activity()
    }
    override fun onStop() {
        if (!isChangingConfigurations) { app.osBiometry.background(); documents.lock(); generator.lock() }
        super.onStop()
    }
    override fun onDestroy() {
        app.osBiometry.detach(this, isChangingConfigurations)
        super.onDestroy()
    }
}
