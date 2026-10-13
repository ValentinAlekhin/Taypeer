package dev.taypeer

import dev.taypeer.bridge.SecretAccessGrant
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import java.util.concurrent.atomic.AtomicLong

internal data class AccessTarget(val database: String, val entry: String)
/** An opaque UI owner/binding stamp, with no session, value or callback retained by the ledger. */
internal data class AccessBinding(val owner: String, val epoch: Long, val generation: ULong)
internal data class AccessAttempt(val id: Long, val target: AccessTarget, val busy: Boolean, val failed: Boolean)
internal data class AccessConfirmation(val target: AccessTarget, val binding: AccessBinding, val time: Long)
internal data class AccessLedgerView(val attempts: List<AccessAttempt> = emptyList())
internal fun interface AccessConfirmer { suspend fun confirm(grant: SecretAccessGrant): Long }

/** Application lifetime, accepted opaque events only. Retrying never obtains or uses a secret. */
internal class SecretAccessState(
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main),
    private val confirmer: AccessConfirmer = AccessConfirmer { grant -> withContext(Dispatchers.IO) { grant.confirm() } },
) : AutoCloseable {
    private data class Accepted(val id: Long, val target: AccessTarget, val binding: AccessBinding, val grant: SecretAccessGrant)
    private sealed interface Command {
        data class Accept(val value: Accepted) : Command
        data class Retry(val id: Long) : Command
    }
    private val ids = AtomicLong()
    private val queue = Channel<Command>(Channel.UNLIMITED)
    private val pending = linkedMapOf<Long, Accepted>()
    private val mutableView = MutableStateFlow(AccessLedgerView())
    private val mutableConfirmations = MutableSharedFlow<AccessConfirmation>()
    val view: StateFlow<AccessLedgerView> get() = mutableView
    val confirmations: SharedFlow<AccessConfirmation> get() = mutableConfirmations

    init {
        scope.launch {
            try {
                for (command in queue) {
                    when (command) {
                        is Command.Accept -> {
                            pending[command.value.id] = command.value
                            confirm(command.value)
                        }
                        is Command.Retry -> pending[command.id]?.let { confirm(it) }
                    }
                }
            } finally {
                pending.values.forEach { it.grant.close() }
                pending.clear()
                while (true) {
                    val command = queue.tryReceive().getOrNull() ?: break
                    if (command is Command.Accept) command.value.grant.close()
                }
            }
        }
    }

    /** Safe from a draw callback: enqueue only, no snapshot writes or I/O. */
    fun accept(target: AccessTarget, binding: AccessBinding, grant: SecretAccessGrant): Boolean =
        queue.trySend(Command.Accept(Accepted(ids.incrementAndGet(), target, binding, grant))).isSuccess

    fun retry(id: Long) { queue.trySend(Command.Retry(id)) }

    private suspend fun confirm(value: Accepted) {
        val before = mutableView.value
        mutableView.value = before.copy(attempts = before.attempts.filter { it.id != value.id } + AccessAttempt(value.id, value.target, true, false))
        val time = try { confirmer.confirm(value.grant) }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) {
            val failed = mutableView.value
            mutableView.value = failed.copy(attempts = failed.attempts.map { if (it.id == value.id) it.copy(busy = false, failed = true) else it })
            return
        }
        pending.remove(value.id)
        value.grant.close()
        val latest = mutableView.value
        // Native returns the latest confirmed event, including when wall clocks move backwards.
        mutableView.value = latest.copy(attempts = latest.attempts.filter { it.id != value.id })
        mutableConfirmations.emit(AccessConfirmation(value.target, value.binding, time))
    }

    override fun close() { queue.close(); scope.cancel() }
}
