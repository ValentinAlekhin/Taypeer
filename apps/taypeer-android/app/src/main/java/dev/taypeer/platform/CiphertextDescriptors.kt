package dev.taypeer.platform

import android.os.ParcelFileDescriptor
import android.system.Os
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.CiphertextFiles

/** Owns already-transferred ciphertext FDs. No path lookup or plaintext staging. */
class CiphertextDescriptors(private val allocateFile: () -> ParcelFileDescriptor) : CiphertextFiles, AutoCloseable {
    private val files = mutableMapOf<ULong, ParcelFileDescriptor>()
    private var next = 1uL
    private var closed = false

    @Synchronized fun accept(file: ParcelFileDescriptor): ULong {
        if (closed || next == ULong.MAX_VALUE) {
            file.close()
            throw AndroidException.InvalidFile()
        }
        val id = next++
        files[id] = file
        return id
    }
    @Synchronized override fun allocate(): ULong = guarded { accept(allocateFile()) }
    @Synchronized override fun length(file: ULong): ULong = guarded {
        val size = Os.fstat(descriptor(file).fileDescriptor).st_size
        if (size < 0) throw AndroidException.InvalidFile()
        size.toULong()
    }
    @Synchronized override fun read(file: ULong, offset: ULong, count: UInt): ByteArray = guarded {
        if (count > 65536u || offset > Long.MAX_VALUE.toULong()) throw AndroidException.InvalidFile()
        val bytes = ByteArray(count.toInt())
        val read = Os.pread(descriptor(file).fileDescriptor, bytes, 0, bytes.size, offset.toLong())
        bytes.copyOf(read)
    }
    @Synchronized override fun write(file: ULong, offset: ULong, bytes: ByteArray): UInt = guarded {
        if (bytes.size > 65536 || offset > Long.MAX_VALUE.toULong()) throw AndroidException.InvalidFile()
        Os.pwrite(descriptor(file).fileDescriptor, bytes, 0, bytes.size, offset.toLong()).toUInt()
    }
    @Synchronized override fun release(file: ULong) {
        try { files.remove(file)?.close() } catch (_: java.io.IOException) {
            // Best-effort resource cleanup is not a commit acknowledgement. Process exit
            // is the final lifetime bound; never unwind Rust Drop through a callback.
        }
    }
    @Synchronized override fun close() {
        closed = true
        val owned = files.values.toList()
        files.clear()
        var failure = false
        owned.forEach { try { it.close() } catch (_: java.io.IOException) { failure = true } }
        if (failure) throw AndroidException.InvalidFile()
    }
    private fun descriptor(id: ULong): ParcelFileDescriptor = files[id] ?: throw AndroidException.InvalidFile()
    private inline fun <T> guarded(block: () -> T): T = try { block() } catch (_: Exception) {
        // Do not expose provider/OS diagnostics across the language boundary.
        throw AndroidException.InvalidFile()
    }
}
