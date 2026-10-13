package dev.taypeer

import android.content.Context
import android.net.Uri
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.SelectedInput
import dev.taypeer.platform.SelectedDescriptors
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ExecutionException

/** Receipt replay can finish without opening a provider that has since disappeared. */
internal class IconSelectedInput(context: Context, uri: Uri) : SelectedInput {
    private var selection: Pair<Context, Uri>? = context.applicationContext to uri
    private var started = false
    private var closed = false
    private val opened = CompletableFuture<SelectedInput>()

    private fun input(): SelectedInput {
        val choice = synchronized(this) {
            if (closed) throw AndroidException.InvalidOptions()
            if (started) null else { started = true; selection ?: throw AndroidException.InvalidOptions() }
        }
        if (choice != null) {
            try {
                val input = SelectedDescriptors.openInput(choice.first, choice.second)
                val accepted = synchronized(this) { selection = null; !closed && opened.complete(input) }
                if (!accepted) { input.close(); throw AndroidException.InvalidOptions() }
            } catch (failure: Exception) {
                synchronized(this) { selection = null; opened.completeExceptionally(failure) }
            }
        }
        return try { opened.get() }
        catch (failure: ExecutionException) { throw (failure.cause as? AndroidException ?: AndroidException.StorageIo()) }
        catch (_: InterruptedException) { Thread.currentThread().interrupt(); throw AndroidException.Runtime() }
    }
    override fun length(): ULong = input().length()
    override fun read(offset: ULong, count: UInt): ByteArray = input().read(offset, count)
    override fun close() {
        val input = synchronized(this) {
            if (closed) return
            closed = true; selection = null
            val value = if (opened.isDone && !opened.isCompletedExceptionally) opened.getNow(null) else null
            opened.completeExceptionally(AndroidException.InvalidOptions())
            value
        }
        input?.close()
    }
}
