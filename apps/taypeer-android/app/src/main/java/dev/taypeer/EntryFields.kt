package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.taypeer.bridge.*

/** Read and edit surfaces share the same retained builders with visual regression fixtures. */
@Composable
internal fun EntrySummary(entry: EntryRow, tags: List<String>, expiresAt: Long?, revealedPassword: String?, togglePassword: () -> Unit, copy: (String) -> Unit, copyPassword: () -> Unit, showProperties: Boolean = true, display: SecretDisplayTicket? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
    ValueRow(R.string.title, entry.title)
    ValueRow(R.string.username, entry.username ?: "—") { entry.username?.let { value -> IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-username")) { copy(value) } } }
    ValueRow(R.string.password, if (entry.hasPassword == true) if (revealedPassword != null) revealedPassword.orEmpty() else "••••••••••••" else "—", secret = true, display = if (revealedPassword != null) display else null) {
        if (entry.hasPassword == true) {
            IconAction(if (revealedPassword == null) R.drawable.ic_eye else R.drawable.ic_eye_off, if (revealedPassword == null) R.string.reveal else R.string.hide, Modifier.testTag("reveal-password")) { togglePassword() }
            IconAction(R.drawable.ic_copy, R.string.copy, Modifier.testTag("copy-password")) { copyPassword() }
        }
    }
    ValueRow(R.string.url, entry.url ?: "—") { entry.url?.let { value -> IconAction(R.drawable.ic_copy, R.string.copy) { copy(value) } } }
    if (showProperties) {
        ValueRow(R.string.tags, tags.joinToString(", ").ifEmpty { "—" })
        ValueRow(R.string.expiry, expiresAt?.let { displayTime(it) } ?: stringResource(R.string.never_expires))
    }
    ValueRow(R.string.notes, entry.notes ?: "—")
    }
}

@Composable
internal fun EntryFields(input: EntryInput, enabled: Boolean, revealed: String, edit: (EntryTextField, String) -> Unit, editTags: (String) -> Unit, enableExpiry: (Boolean) -> Unit, editExpiry: (String) -> Unit, reveal: () -> Unit, hide: () -> Unit, generator: () -> Unit, favicon: (() -> Unit)? = null, display: SecretDisplayTicket? = null) {
    var showPassword by remember(LocalSecretMaskEpoch.current) { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Field(input.title, R.string.title, "entry-title", enabled) { edit(EntryTextField.TITLE, it) }
            Field(input.username, R.string.username, "entry-username", enabled) { edit(EntryTextField.USERNAME, it) }
            val passwordValue = if (showPassword && input.password.isEmpty() && input.source.hasPassword) revealed else input.password
            val savedPasswordDisplay = if (showPassword && input.password.isEmpty() && input.source.hasPassword && revealed.isNotEmpty()) display else null
            PlainField(passwordValue, R.string.password, "entry-password", enabled = enabled,
                structured = true, transformation = if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
                keyboard = KeyboardOptions(keyboardType = KeyboardType.Password), placeholder = if (input.source.hasPassword) "••••••••••••" else null,
                actions = {
                    IconAction(if (showPassword) R.drawable.ic_eye_off else R.drawable.ic_eye, if (showPassword) R.string.hide else R.string.reveal, Modifier.testTag("reveal-entry-input-password"), enabled = enabled) {
                        showPassword = !showPassword
                        if (showPassword && input.password.isEmpty() && input.source.hasPassword) reveal() else hide()
                    }
                    IconAction(R.drawable.ic_dice_5, R.string.generator, Modifier.testTag("open-generator"), enabled = enabled) { generator() }
                }, inputModifier = Modifier.secretDisplay(savedPasswordDisplay), update = { edit(EntryTextField.PASSWORD, it) })
            PlainField(input.url, R.string.url, "entry-url", enabled = enabled, structured = true, actions = {
                if (favicon != null) IconAction(R.drawable.ic_download, R.string.download_favicon, Modifier.testTag("download-favicon"), enabled = enabled && input.url.isNotEmpty(), action = favicon)
            }, update = { edit(EntryTextField.URL, it) })
            Field(input.tags, R.string.tags, "entry-tags", enabled, false) { editTags(it) }
            ExpiryField(input, enableExpiry, editExpiry, enabled)
            Field(input.notes, R.string.notes, "entry-notes", enabled, false) { edit(EntryTextField.NOTES, it) }
    }
}
