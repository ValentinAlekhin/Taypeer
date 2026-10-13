package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

@Composable
internal fun SecretAccessStatus(owner: SecretAccessState, database: String?) {
    val view by owner.view.collectAsState()
    val failed = view.attempts.filter { it.target.database == database && it.failed }
    if (failed.isEmpty()) return
    Column(Modifier.fillMaxWidth().padding(16.dp).testTag("access-metadata-error"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.access_record_failed), color = MaterialTheme.colorScheme.error)
        FlatAction(R.string.retry, Modifier.fillMaxWidth().testTag("retry-access")) { failed.forEach { owner.retry(it.id) } }
    }
}

@Composable
internal fun SecretAccessContainer(owner: SecretAccessState, database: String?, content: @Composable () -> Unit) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val retryPanelHeight = maxHeight / 3
        Column(Modifier.fillMaxSize()) {
            // The retry panel remains scrollable without consuming the history route at large fonts.
            Box(Modifier.heightIn(max = retryPanelHeight).verticalScroll(rememberScrollState())) {
                SecretAccessStatus(owner, database)
            }
            Box(Modifier.weight(1f).fillMaxWidth()) { content() }
        }
    }
}
