package dev.taypeer.platform

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.os.ParcelFileDescriptor
import android.os.Process
import dev.taypeer.bridge.WorkerStreams
import dev.taypeer.bridge.serveDocument

/** One real document generation. Only private pipes and transferred ciphertext are reachable. */
class DocumentService : Service() {
    private var owner: IBinder? = null
    private val ownerDied = IBinder.DeathRecipient { Process.killProcess(Process.myPid()) }
    private val endpoint = object : IDocumentProcess.Stub() {
        @Synchronized
        override fun initialize(host: IBinder, commands: ParcelFileDescriptor, responses: ParcelFileDescriptor, persistence: ICipherPersistence, selected: ISelectedTransfers): Int {
            check(owner == null)
            host.linkToDeath(ownerDied, 0)
            owner = host
            Thread({
                try {
                    CiphertextDescriptors { temporaryCiphertext(persistence) }.use { files ->
                        PipeStreams(commands, responses).use { streams ->
                            serveDocument(streams, BinderDocumentPersistence(persistence, files), files, BinderSelectedTransfers(selected))
                        }
                    }
                } catch (_: Exception) {
                    // Native errors travel on the framed response pipe; never log document data.
                } finally { Process.killProcess(Process.myPid()) }
            }, "document-worker").start()
            return Process.myPid()
        }
        override fun terminate() { Process.killProcess(Process.myPid()) }
    }
    override fun onBind(intent: Intent): IBinder = endpoint
    override fun onUnbind(intent: Intent): Boolean {
        Process.killProcess(Process.myPid())
        return false
    }
}

/** Blocking private FD I/O is independent from Binder death and termination control. */
internal class PipeStreams(commands: ParcelFileDescriptor, responses: ParcelFileDescriptor) : WorkerStreams, AutoCloseable {
    private val input = ParcelFileDescriptor.AutoCloseInputStream(commands)
    private val output = ParcelFileDescriptor.AutoCloseOutputStream(responses)
    override fun read(count: UInt): ByteArray {
        if (count > 65536u) throw dev.taypeer.bridge.AndroidException.InvalidFile()
        if (count == 0u) return ByteArray(0)
        val bytes = ByteArray(count.toInt())
        return try { val read = input.read(bytes); if (read < 0) ByteArray(0) else bytes.copyOf(read) }
        catch (_: Exception) { throw dev.taypeer.bridge.AndroidException.Runtime() }
        finally { bytes.fill(0) }
    }
    override fun write(bytes: ByteArray): UInt {
        if (bytes.size > 65536) throw dev.taypeer.bridge.AndroidException.InvalidFile()
        return try { output.write(bytes); bytes.size.toUInt() }
        catch (_: Exception) { throw dev.taypeer.bridge.AndroidException.Runtime() }
        finally { bytes.fill(0) }
    }
    override fun close() {
        try { input.close() } finally { output.close() }
    }
}
