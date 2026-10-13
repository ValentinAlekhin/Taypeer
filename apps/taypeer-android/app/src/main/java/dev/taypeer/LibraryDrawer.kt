package dev.taypeer

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/** The product and visual harness retain the same drawer overlay and measured width. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun LibraryDrawer(drawerState: DrawerState, databaseOpen: Boolean, gesturesEnabled: Boolean,
    drawerContent: @Composable () -> Unit, content: @Composable () -> Unit) {
    BoxWithConstraints {
        val width = (maxWidth - 48.dp).coerceAtMost(342.dp)
        ModalNavigationDrawer(drawerState = drawerState, gesturesEnabled = gesturesEnabled,
            scrimColor = Color.Black.copy(alpha = .65f), drawerContent = {
                if (databaseOpen) ModalDrawerSheet(Modifier.width(width).fillMaxHeight()
                    .testTag(if (drawerState.isOpen) "database-drawer-open" else "database-drawer-closed"),
                    drawerShape = RectangleShape, drawerContainerColor = MaterialTheme.colorScheme.surfaceContainer) { drawerContent() }
            }, content = content)
    }
}
