package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.taypeer.qr.QrScannerScreen

/** Ephemeral bearer code; never saved in Activity state or local preferences. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReceiveInvitationScreen(
    onJoin: (String) -> Unit, onClose: () -> Unit,
    scanner: @Composable ((String) -> Unit, () -> Unit) -> Unit = { deliver, close -> QrScannerScreen(deliver, close) },
) {
    var code by remember { mutableStateOf("") }
    var scanning by remember { mutableStateOf(false) }
    val stackedActions = LocalDensity.current.fontScale >= 1.8f
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) { code = ""; scanning = false }
        }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }
    BackHandler { if (scanning) scanning = false else onClose() }
    if (scanning) {
        scanner({ code = it; scanning = false }, { scanning = false })
        return
    }
    Scaffold(topBar = { PhoneBar(stringResource(R.string.receive_database), back = onClose) }, bottomBar = {
        Column {
            HorizontalDivider()
            val actions = Modifier.fillMaxWidth().navigationBarsPadding().imePadding().padding(horizontal = 16.dp, vertical = 8.dp)
            if (stackedActions) Column(actions, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                FlatAction(R.string.cancel, Modifier.fillMaxWidth().testTag("receive-cancel"), action = onClose)
                FlatAction(R.string.connect_database, Modifier.fillMaxWidth().testTag("receive-connect"), enabled = code.isNotBlank()) { onJoin(code); code = "" }
            } else Row(actions, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FlatAction(R.string.cancel, Modifier.weight(1f).testTag("receive-cancel"), action = onClose)
                FlatAction(R.string.connect_database, Modifier.weight(1f).testTag("receive-connect"), enabled = code.isNotBlank()) { onJoin(code); code = "" }
            }
        }
    }) { insets ->
        Column(Modifier.fillMaxSize().padding(insets).padding(16.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            PlainField(code, R.string.invitation_code, "invitation-code", singleLine = false, minLines = 4, maxLines = 8) { if (it.length <= 16_384) code = it }
            FlatAction(R.string.scan_qr, Modifier.fillMaxWidth().testTag("scan-invitation")) { scanning = true }
            Text(stringResource(R.string.receive_invitation_hint), style = MaterialTheme.typography.bodyMedium)
        }
    }
}
