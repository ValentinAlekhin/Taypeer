package dev.taypeer

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/** Date selection only formats user input; Rust validates and owns the saved expiration. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ExpiryField(input: EntryInput, enable: (Boolean) -> Unit, update: (String) -> Unit, enabled: Boolean) {
    var choosing by remember { mutableStateOf(false) }
    val date = input.expiry.toLongOrNull()
    val title = if (!input.expiryEnabled || date == null) stringResource(R.string.never_expires)
        else DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(androidx.compose.ui.platform.LocalConfiguration.current.locales[0]).withZone(ZoneId.systemDefault()).format(Instant.ofEpochMilli(date))
    Column(Modifier.fillMaxWidth()) {
        Text(stringResource(R.string.expiry), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("entry-expiry").clickable(enabled = enabled) { choosing = true }, verticalAlignment = Alignment.CenterVertically) {
            Text(title, Modifier.weight(1f))
            IconAction(R.drawable.ic_chevron_down, R.string.expiry, enabled = enabled) { choosing = true }
        }
        HorizontalDivider()
    }
    if (choosing) {
        val picker = rememberDatePickerState(initialSelectedDateMillis = if (input.expiryEnabled) date else null)
        DatePickerDialog(onDismissRequest = { choosing = false }, confirmButton = {
            TextButton({ picker.selectedDateMillis?.let { enable(true); update(it.toString()) }; choosing = false }, enabled = picker.selectedDateMillis != null) { Text(stringResource(R.string.apply)) }
        }, dismissButton = {
            Row { TextButton({ enable(false); choosing = false }) { Text(stringResource(R.string.never_expires)) }; TextButton({ choosing = false }) { Text(stringResource(R.string.cancel)) } }
        }) { DatePicker(picker) }
    }
}
