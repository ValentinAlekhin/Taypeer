package dev.taypeer

import android.os.Bundle
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.ui.res.stringResource

class MainActivity : ComponentActivity() {
    private val app get() = application as TaypeerApplication
    private val generator by viewModels<GeneratorState>()
    private val documents by viewModels<DocumentState>()
    internal val documentState get() = documents

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        setContent {
            TaypeerTheme {
                val attachments = rememberAttachmentActions(documents)
                Surface(Modifier.fillMaxSize()) {
                    if (documents.generator) Column {
                        TextButton({ documents.generator = false }) { Text(stringResource(R.string.library)) }
                        if (documents.entry != null && generator.generated.isNotEmpty()) TextButton({
                            documents.edit(dev.taypeer.bridge.EntryTextField.PASSWORD, generator.generated)
                            generator.lock(); documents.generator = false
                        }) { Text(stringResource(R.string.use_password)) }
                        GeneratorScreen(generator) { value ->
                            try { app.clipboard.copy(value); generator.error = false }
                            catch (_: Exception) { generator.error = true }
                        }
                    } else LibraryScreen(documents, attachments) { value ->
                        try {
                            app.clipboard.copy(value)
                            Toast.makeText(this@MainActivity, R.string.clipboard_copied, Toast.LENGTH_SHORT).show()
                        } catch (_: Exception) { documents.reportFailure() }
                    }
                }
            }
        }
    }
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        try { app.clipboard.focusChanged(hasFocus) } catch (_: Exception) { generator.error = true }
        if (!hasFocus) documents.hideSecrets()
    }
    override fun onUserInteraction() {
        super.onUserInteraction()
        documents.activity()
    }
    override fun onStop() {
        if (!isChangingConfigurations) { documents.lock(); generator.lock() }
        super.onStop()
    }
}
