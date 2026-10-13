package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.EntryRow
import dev.taypeer.bridge.SecretAccessGrant
import dev.taypeer.bridge.SecretAccessView

enum class ComparisonSide { SAVED, CURRENT }
private data class ComparedRevision(val entry: String, val revision: String)
private data class ComparedEntries(val saved: EntryRow, val current: EntryRow)
private data class ComparedPassword(val access: SecretAccessView, val current: EntryRow?)

/** Two masked projections. Each protected side requires its own explicit user action. */
class EntryComparisonState internal constructor(private val commands: SessionCommands,
    private val ticket: (String, SecretAccessGrant, () -> Boolean) -> SecretDisplayTicket? = { _, grant, _ -> grant.close(); null }) {
    var showing by mutableStateOf(false); private set
    var saved by mutableStateOf<EntryRow?>(null); private set
    var current by mutableStateOf<EntryRow?>(null); private set
    var savedPassword by mutableStateOf<String?>(null); private set
    var currentPassword by mutableStateOf<String?>(null); private set
    var busy by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    private var selected: ComparedRevision? = null
    private var routeEpoch = 0L
    private var secretEpoch = 0L
    private var refreshQueued = false
    private var savedDisplay by mutableStateOf<SecretDisplayTicket?>(null)
    private var currentDisplay by mutableStateOf<SecretDisplayTicket?>(null)
    private var savedDisplayIdentity: Any? = null
    private var currentDisplayIdentity: Any? = null
    internal fun display(side: ComparisonSide) = if (side == ComparisonSide.SAVED) savedDisplay else currentDisplay

    fun open(entry: String, revision: String) {
        if (busy) return
        close()
        val target = ComparedRevision(entry, revision)
        selected = target; showing = true
        execute(command = { session -> ComparedEntries(session.revision(entry, revision), session.entry(entry)) }) {
            saved = it.saved; current = it.current
        }
    }

    fun retry() { selected?.let { open(it.entry, it.revision) } }

    private fun <T> execute(command: (DocumentSession) -> T, discarded: (T) -> Unit = {}, completed: (T) -> Unit) {
        if (busy || !showing) return
        val expectedRoute = routeEpoch
        busy = true; error = false
        val accepted = commands.execute(navigate = false, command = command, started = {},
            completed = { value ->
                if (showing && routeEpoch == expectedRoute) { busy = false; completed(value); refreshAfterCommand() }
                else discarded(value)
            }, failed = {
                if (showing && routeEpoch == expectedRoute) { busy = false; error = true; refreshAfterCommand() }
            }, discarded = discarded)
        if (!accepted && routeEpoch == expectedRoute) { busy = false; error = true }
    }

    private fun readPassword(side: ComparisonSide, completed: (SecretAccessView, () -> Boolean) -> Unit) {
        val target = selected ?: return
        val view = if (side == ComparisonSide.SAVED) saved else current
        if (view?.hasPassword != true) return
        val expectedSecret = secretEpoch
        val expectedRoute = routeEpoch
        execute(command = { session ->
            val value = session.revealForAccess(target.entry, if (side == ComparisonSide.SAVED) target.revision else null, null)
            try { ComparedPassword(value, if (side == ComparisonSide.CURRENT) session.entry(target.entry) else null) }
            catch (failure: Exception) { value.grant.close(); throw failure }
        }, discarded = { it.access.grant.close() }) {
            if (selected == target && secretEpoch == expectedSecret) {
                it.current?.let { view -> current = view }
                completed(it.access) { showing && routeEpoch == expectedRoute && selected == target && secretEpoch == expectedSecret }
            } else it.access.grant.close()
        }
    }

    fun reveal(side: ComparisonSide) {
        if (busy) return
        val shown = if (side == ComparisonSide.SAVED) savedPassword else currentPassword
        if (shown != null) {
            secretEpoch++
            if (side == ComparisonSide.SAVED) { savedDisplay?.discard(); savedDisplay = null; savedDisplayIdentity = null; savedPassword = null }
            else { currentDisplay?.discard(); currentDisplay = null; currentDisplayIdentity = null; currentPassword = null }
        } else readPassword(side) { access, allowed ->
            if (!allowed()) { access.grant.close(); return@readPassword }
            val target = selected!!
            val expectedRoute = routeEpoch
            val identity = Any()
            if (side == ComparisonSide.SAVED) {
                savedDisplay?.discard(); savedDisplayIdentity = identity
                savedDisplay = ticket(target.entry, access.grant) {
                    showing && routeEpoch == expectedRoute && selected == target && savedDisplayIdentity === identity
                }
                savedPassword = access.value
            } else {
                currentDisplay?.discard(); currentDisplayIdentity = identity
                currentDisplay = ticket(target.entry, access.grant) {
                    showing && routeEpoch == expectedRoute && selected == target && currentDisplayIdentity === identity
                }
                currentPassword = access.value
            }
        }
    }

    fun copyPassword(side: ComparisonSide, copy: (String) -> Boolean) = readPassword(side) { access, allowed ->
        val display = ticket(selected!!.entry, access.grant, allowed)
        try { if (allowed() && copy(access.value)) { if (display?.acceptCopy() == false) display.discard() } else display?.discard() }
        catch (failure: Exception) { display?.discard(); throw failure }
    }

    fun hideSecrets() {
        secretEpoch++; savedDisplayIdentity = null; currentDisplayIdentity = null
        savedDisplay?.discard(); currentDisplay?.discard(); savedDisplay = null; currentDisplay = null
        savedPassword = null; currentPassword = null
    }

    internal fun refreshIfOpen() {
        if (!showing) return
        hideSecrets()
        if (busy) { refreshQueued = true; return }
        val target = selected ?: return
        execute(command = { it.entry(target.entry) }) { current = it }
    }

    private fun refreshAfterCommand() {
        if (refreshQueued) { refreshQueued = false; refreshIfOpen() }
    }

    fun close() {
        routeEpoch++; hideSecrets(); selected = null
        showing = false; busy = false; error = false; saved = null; current = null; refreshQueued = false
    }
}
