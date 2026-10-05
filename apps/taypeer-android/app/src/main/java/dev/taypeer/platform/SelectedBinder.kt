package dev.taypeer.platform

import android.os.Binder
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.SelectedTransfersHost
import dev.taypeer.bridge.SelectedTransfersRemote

private fun selectedId(id: Long): ULong {
    if (id <= 0) throw AndroidException.InvalidOptions()
    return id.toULong()
}
private fun selectedOffset(offset: Long): ULong {
    if (offset < 0) throw AndroidException.InvalidOptions()
    return offset.toULong()
}
private fun selectedLong(value: ULong): Long {
    if (value > Long.MAX_VALUE.toULong()) throw AndroidException.InvalidOptions()
    return value.toLong()
}
private fun selectedCount(count: Int): UInt {
    if (count !in 0..SELECTED_CHUNK) throw AndroidException.InvalidOptions()
    return count.toUInt()
}
private inline fun <T> hostSelection(block: () -> T): T {
    // The app owns SAF grants; local providers must not see the isolated caller UID.
    // Native still checks generation/selection before invoking any provider callback.
    val identity = Binder.clearCallingIdentity()
    try { return block() } finally { Binder.restoreCallingIdentity(identity) }
}

/** Host retains the native generation/draft-scoped registry; Binder grants no paths. */
internal class SelectedTransfersEndpoint(private val selected: SelectedTransfersHost) : ISelectedTransfers.Stub() {
    override fun read(id: Long, offset: Long, count: Int): SelectedChunk = try {
        val bytes = hostSelection { selected.read(selectedId(id), selectedOffset(offset), selectedCount(count)) }
        if (bytes.size > count) { bytes.fill(0); throw AndroidException.InvalidFile() }
        SelectedChunk(0, bytes)
    } catch (error: Exception) { SelectedChunk(code(error), null) }
    override fun write(id: Long, offset: Long, bytes: ByteArray): SelectedWriteResult = try {
        selectedCount(bytes.size)
        val written = hostSelection { selected.write(selectedId(id), selectedOffset(offset), bytes) }
        if (written > bytes.size.toUInt()) throw AndroidException.InvalidFile()
        SelectedWriteResult(0, written.toInt())
    } catch (error: Exception) { SelectedWriteResult(code(error), 0) }
    finally { bytes.fill(0) }
    override fun finish(id: Long): Int = status { hostSelection { selected.finish(selectedId(id)) } }
}

/** Worker sees bounded private capabilities only; lost write/finish replies cannot mean success. */
internal class BinderSelectedTransfers(private val host: ISelectedTransfers) : SelectedTransfersRemote {
    override fun read(id: ULong, offset: ULong, count: UInt): ByteArray = remote {
        if (count > SELECTED_CHUNK.toUInt()) throw AndroidException.InvalidOptions()
        val result = host.read(selectedLong(id), selectedLong(offset), count.toInt())
        try {
            checked(result.status)
            val bytes = result.bytes ?: throw AndroidException.InvalidFile()
            if (bytes.size > count.toInt()) throw AndroidException.InvalidFile()
            bytes
        } catch (error: Exception) { result.bytes?.fill(0); throw error }
    }
    override fun write(id: ULong, offset: ULong, bytes: ByteArray): UInt = try {
        mutation {
            selectedCount(bytes.size)
            val result = host.write(selectedLong(id), selectedLong(offset), bytes)
            checked(result.status)
            if (result.written !in 0..bytes.size) throw AndroidException.InvalidFile()
            result.written.toUInt()
        }
    } finally { bytes.fill(0) }
    override fun finish(id: ULong) = mutation { checked(host.finish(selectedLong(id))) }
}
