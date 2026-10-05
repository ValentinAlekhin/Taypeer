package dev.taypeer.platform

import android.content.Context
import android.net.Uri
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.system.Os
import android.system.OsConstants
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.SelectedInput
import dev.taypeer.bridge.SelectedOutput
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ExecutionException

/** SAF is accessed only after an explicit selection and off the UI thread. No URI is retained. */
object SelectedDescriptors {
    fun openInput(context: Context, uri: Uri): SelectedInput {
        offUi()
        return SelectedDescriptorInput(selectedOpen(context, uri, "r"))
    }
    fun openOutput(context: Context, uri: Uri): SelectedOutput {
        offUi()
        return LazySelectedOutput(context.applicationContext, uri)
    }
    fun displayName(context: Context, uri: Uri): String? {
        offUi()
        return selectedIo {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (index < 0 || !cursor.moveToFirst() || cursor.isNull(index)) return@use null
                cursor.getString(index).takeIf { it.isNotEmpty() && it.length <= 65536 }
            }
        }
    }
    private fun offUi() {
        if (Looper.myLooper() == Looper.getMainLooper()) throw AndroidException.Runtime()
    }
}

/** No output FD exists until the worker has checked the selected blob's visibility. */
private class LazySelectedOutput(context: Context, uri: Uri) : SelectedOutput {
    private var selection: Pair<Context, Uri>? = context to uri
    private var started = false
    private var closed = false
    private val opened = CompletableFuture<SelectedDescriptorOutput>()

    private fun output(): SelectedDescriptorOutput {
        val choice = synchronized(this) {
            if (closed) throw AndroidException.InvalidOptions()
            if (started) null else { started = true; selection ?: throw AndroidException.InvalidOptions() }
        }
        if (choice != null) {
            try {
                // Provider I/O and close never run under the ownership lock.
                val destination = SelectedDescriptorOutput(selectedOpen(choice.first, choice.second, "wt"))
                val accepted = synchronized(this) {
                    selection = null
                    !closed && opened.complete(destination)
                }
                if (!accepted) { destination.close(); throw AndroidException.InvalidOptions() }
            } catch (error: Exception) {
                synchronized(this) { selection = null; opened.completeExceptionally(error) }
            }
        }
        return try { opened.get() }
        catch (error: ExecutionException) { throw (error.cause as? AndroidException ?: AndroidException.StorageIo()) }
        catch (_: InterruptedException) { Thread.currentThread().interrupt(); throw AndroidException.Runtime() }
    }
    override fun write(offset: ULong, bytes: ByteArray): UInt = try { output().write(offset, bytes) }
        finally { bytes.fill(0) }
    override fun finish() = output().finish()
    override fun close() {
        val destination = synchronized(this) {
            if (closed) return
            closed = true
            selection = null
            val current = if (opened.isDone && !opened.isCompletedExceptionally) opened.getNow(null) else null
            opened.completeExceptionally(AndroidException.InvalidOptions())
            current
        }
        destination?.close()
    }
}

/** Duplicate under a short ownership lock; provider I/O never holds that lock. */
private class SelectedDescriptor(file: ParcelFileDescriptor) {
    private var file: ParcelFileDescriptor? = file
    fun <T> withFile(block: (ParcelFileDescriptor) -> T): T {
        val duplicate = selectedIo {
            synchronized(this) {
                val current = file ?: throw AndroidException.InvalidOptions()
                ParcelFileDescriptor.dup(current.fileDescriptor)
            }
        }
        return selectedIo { duplicate.use { block(it) } }
    }
    fun close() {
        val removed = synchronized(this) { file.also { file = null } }
        try { removed?.close() } catch (_: Exception) { /* Cleanup is never a successful durability acknowledgement. */ }
    }
}

internal class SelectedDescriptorInput(file: ParcelFileDescriptor) : SelectedInput {
    private val descriptor = SelectedDescriptor(file)
    private val bytes: ULong
    init {
        try {
            bytes = descriptor.withFile { selected ->
                val known = selected.statSize
                if (known >= 0) known.toULong() else {
                    // A pipe of unknown size cannot satisfy quota-before-read without copying plaintext.
                    val original = Os.lseek(selected.fileDescriptor, 0, OsConstants.SEEK_CUR)
                    val end = try { Os.lseek(selected.fileDescriptor, 0, OsConstants.SEEK_END) }
                        finally { Os.lseek(selected.fileDescriptor, original, OsConstants.SEEK_SET) }
                    if (end < 0) throw AndroidException.InvalidFile()
                    end.toULong()
                }
            }
        } catch (error: Exception) { descriptor.close(); throw error }
    }
    override fun length(): ULong = descriptor.withFile { bytes }
    override fun read(offset: ULong, count: UInt): ByteArray {
        val position = selectedPosition(offset)
        if (count > SELECTED_CHUNK.toUInt() || offset > bytes) throw AndroidException.InvalidOptions()
        // The shared BlobStore probes one byte past the declared size to reject a
        // provider that grew after selection. Do not turn that probe into false EOF.
        val capacity = minOf(count.toULong(), bytes - offset + 1uL).toInt()
        return descriptor.withFile { selected ->
            val buffer = ByteArray(capacity)
            try {
                if (capacity == 0) ByteArray(0)
                else buffer.copyOf(Os.pread(selected.fileDescriptor, buffer, 0, capacity, position))
            } finally { buffer.fill(0) }
        }
    }
    override fun close() = descriptor.close()
}

internal class SelectedDescriptorOutput(file: ParcelFileDescriptor) : SelectedOutput {
    private val descriptor = SelectedDescriptor(file)
    override fun write(offset: ULong, bytes: ByteArray): UInt = try {
        val position = selectedPosition(offset)
        if (bytes.size > SELECTED_CHUNK) throw AndroidException.InvalidOptions()
        descriptor.withFile { selected ->
            Os.pwrite(selected.fileDescriptor, bytes, 0, bytes.size, position).toUInt()
        }
    } finally { bytes.fill(0) }
    override fun finish() = descriptor.withFile { selected -> Os.fsync(selected.fileDescriptor) }
    override fun close() = descriptor.close()
}

private fun selectedPosition(offset: ULong): Long {
    if (offset > Long.MAX_VALUE.toULong()) throw AndroidException.InvalidOptions()
    return offset.toLong()
}
private fun selectedOpen(context: Context, uri: Uri, mode: String): ParcelFileDescriptor = selectedIo {
    context.contentResolver.openFileDescriptor(uri, mode) ?: throw AndroidException.StorageIo()
}
private inline fun <T> selectedIo(block: () -> T): T = try { block() }
    catch (error: AndroidException) { throw error }
    catch (_: Exception) { throw AndroidException.StorageIo() }
