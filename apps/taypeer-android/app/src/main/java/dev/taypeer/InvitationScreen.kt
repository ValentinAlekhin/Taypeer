package dev.taypeer

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

/** The bearer stays in the current unlocked presentation and uses the protected clipboard callback. */
@Composable
internal fun InvitationScreen(code: String, databaseName: String?, onClose: () -> Unit, copy: (String) -> Unit, newInvitation: (() -> Unit)? = null) {
    BackHandler(onBack = onClose)
    Scaffold(topBar = { PhoneBar(stringResource(R.string.invite_device_title), back = onClose) }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp).testTag("invitation-screen"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(listOfNotNull(databaseName?.let { stringResource(R.string.invitation_database, it) },
                stringResource(if (code.isEmpty()) R.string.invitation_expired else R.string.invitation_expires)).joinToString("\n"),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (code.isNotEmpty()) {
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                    InvitationQr(code, Modifier.size(240.dp).testTag("invitation-qr"))
                }
                ReadOnlyField(code, R.string.invitation_code, "invitation-code", 112.dp)
                FlatAction(R.string.copy_invitation, Modifier.fillMaxWidth().testTag("copy-invitation")) { copy(code) }
                Text(stringResource(R.string.invitation_approval_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            } else newInvitation?.let { FlatAction(R.string.new_invitation, Modifier.fillMaxWidth(), action = it) }
        }
    }
}
