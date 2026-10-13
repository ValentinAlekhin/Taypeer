package dev.taypeer

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import dev.taypeer.bridge.SecretAccessGrant
import java.util.concurrent.atomic.AtomicReference

/** One-shot ownership of a not-yet-displayed grant, never of its secret value. */
internal class SecretDisplayTicket(grant: SecretAccessGrant, private val permitted: () -> Boolean,
    private val enqueue: (SecretAccessGrant) -> Boolean) {
    private val pending = AtomicReference<SecretAccessGrant?>(grant)
    fun accept(): Boolean {
        if (!permitted()) return false
        return enqueueOwned()
    }
    /** A true clipboard ACK proves the action already happened, even if that action hid or locked UI. */
    fun acceptCopy(): Boolean = enqueueOwned()
    private fun enqueueOwned(): Boolean {
        val grant = pending.getAndSet(null) ?: return false
        if (enqueue(grant)) return true
        // A closed queue cannot publish. Retain ownership for discard outside drawing.
        pending.set(grant)
        return false
    }
    fun discard() { pending.getAndSet(null)?.close() }
}

private class DrawnSecret(private val ticket: SecretDisplayTicket) {
    var coordinates: LayoutCoordinates? = null
    fun report() {
        val placed = coordinates?.takeIf { it.isAttached } ?: return
        // boundsInWindow clips through ancestors; an eagerly composed offscreen row is not an access.
        val bounds = placed.boundsInWindow()
        val viewport = placed.findRootCoordinates().boundsInWindow()
        if (bounds.width > 0f && bounds.height > 0f && bounds.overlaps(viewport)) ticket.accept()
    }
}

/** Runs after the actual secret Text draws. The report only guards and enqueues an opaque event. */
@Composable
internal fun Modifier.secretDisplay(ticket: SecretDisplayTicket?): Modifier {
    if (ticket == null) return this
    val drawn = remember(ticket) { DrawnSecret(ticket) }
    return onGloballyPositioned { drawn.coordinates = it }.drawWithContent { drawContent(); drawn.report() }
}
