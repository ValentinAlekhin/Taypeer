package dev.taypeer.platform

import android.os.Build
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.Parcel
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.CipherCommit
import dev.taypeer.bridge.CipherSeed
import dev.taypeer.bridge.CipherSnapshot
import dev.taypeer.bridge.CipherWriter
import dev.taypeer.bridge.DocumentPersistence

private const val MAX_METADATA = 1024 * 1024
private const val MAX_OBJECTS = 4096

private inline fun snapshotResult(block: () -> Bundle): Bundle = try { block() }
    catch (error: Exception) { Bundle().apply { putInt("status", code(error)) } }

internal fun temporaryCiphertext(host: ICipherPersistence): ParcelFileDescriptor = remote {
    val result = host.createTemporary()
    try { checked(result.status); result.file ?: throw AndroidException.InvalidFile() }
    catch (error: Exception) { result.file?.close(); throw error }
}

private fun metadata(bytes: ByteArray): ByteArray {
    if (bytes.size > MAX_METADATA) throw AndroidException.InvalidFile()
    return bytes
}
private fun identifier(value: String): String {
    if (value.isEmpty() || value.length > 256) throw AndroidException.InvalidFile()
    return value
}

/** Binder owns returned duplicates; the table's outgoing lease ends before publication. */
private fun snapshotBundle(view: CipherSnapshot, transfer: (ULong) -> ParcelFileDescriptor): Bundle = Bundle().apply {
    putInt("status", 0)
    putParcelable("file", transfer(view.file))
    putString("fingerprint", view.fingerprint)
    putString("root", view.root)
    putString("working_copy", view.workingCopy)
}
@Suppress("DEPRECATION")
private fun snapshot(bundle: Bundle, files: CiphertextDescriptors): CipherSnapshot {
    val descriptor = if (Build.VERSION.SDK_INT >= 33) bundle.getParcelable("file", ParcelFileDescriptor::class.java)
        else bundle.getParcelable<ParcelFileDescriptor>("file")
    if (descriptor == null) { checked(bundle.getInt("status", 6)); throw AndroidException.InvalidFile() }
    val file = descriptor
    try {
        checked(bundle.getInt("status", 6))
        if (bundle.keySet() != setOf("status", "file", "fingerprint", "root", "working_copy")) throw AndroidException.InvalidFile()
        val fingerprint = identifier(bundle.getString("fingerprint") ?: throw AndroidException.InvalidFile())
        val root = identifier(bundle.getString("root") ?: throw AndroidException.InvalidFile())
        val copy = identifier(bundle.getString("working_copy") ?: throw AndroidException.InvalidFile())
        return CipherSnapshot(files.accept(file), fingerprint, root, copy)
    } catch (error: Exception) { file.close(); throw error }
}

/** Host-owned endpoint. Only verified encrypted objects enter the native coordinator. */
internal class CipherPersistenceEndpoint(
    private val writer: CipherWriter,
    private val files: CiphertextDescriptors,
    private val allocate: () -> ParcelFileDescriptor,
) : ICipherPersistence.Stub() {
    private val outgoing = ThreadLocal<MutableList<ParcelFileDescriptor>>()
    // Bundle serialization does not promise forwarding RETURN_VALUE to nested PFDs.
    // Close every sender duplicate after the framework has copied it into the reply Parcel.
    override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
        outgoing.set(mutableListOf())
        try { return super.onTransact(code, data, reply, flags) }
        finally {
            outgoing.get()?.forEach { try { it.close() } catch (_: Exception) {} }
            outgoing.remove()
        }
    }
    private fun send(file: ParcelFileDescriptor): ParcelFileDescriptor {
        outgoing.get()?.add(file)
        return file
    }
    private fun transfer(id: ULong): ParcelFileDescriptor = try { send(files.transfer(id)) }
        catch (error: Exception) { files.release(id); throw error }
    override fun createTemporary(): CipherFileResult = try { CipherFileResult(0, send(allocate())) } catch (error: Exception) { CipherFileResult(code(error), null) }
    override fun snapshot(): Bundle = snapshotResult { snapshotBundle(writer.snapshot(), ::transfer) }
    override fun create(controls: ByteArray, objects: Array<ParcelFileDescriptor>, checkpoint: String, baseline: String): Int = status {
        consume(objects, { ids -> CipherSeed(metadata(controls), ids, identifier(checkpoint), identifier(baseline)) }, writer::create)
    }
    override fun commit(expected: String, control: String, controls: ByteArray, objects: Array<ParcelFileDescriptor>, remove: Array<String>, checkpoint: String, baseline: String, journal: ByteArray): Bundle = snapshotResult {
        consume(objects, { ids ->
            if (remove.size > MAX_OBJECTS) throw AndroidException.InvalidFile()
            CipherCommit(identifier(expected), identifier(control), metadata(controls), ids,
                remove.map(::identifier), identifier(checkpoint), identifier(baseline), metadata(journal))
        }, { request ->
            val committed = writer.commit(request) // Preserve explicit native errors before success.
            try { snapshotBundle(committed, ::transfer) }
            catch (_: Exception) { throw AndroidException.CommitUncertain() }
        })
    }
    override fun author(authenticated: String?): CipherAuthorResult = try { CipherAuthorResult(0, writer.author(authenticated?.let(::identifier))) } catch (error: Exception) { CipherAuthorResult(code(error), null) }
    override fun transportPublic(): String = writer.transportPublic()
    override fun saveDraft(file: ParcelFileDescriptor): Int = status {
        val id = files.accept(file)
        writer.saveDraft(id) // Native source/Lease consumes this ID, including native failure.
    }
    override fun loadDraft(): CipherFileResult = try { CipherFileResult(0, writer.loadDraft()?.let(::transfer)) } catch (error: Exception) { CipherFileResult(code(error), null) }
    override fun discardDraft(): Int = status { writer.discardDraft() }

    private inline fun <T, R> consume(incoming: Array<ParcelFileDescriptor>, prepare: (List<ULong>) -> T, native: (T) -> R): R {
        val ids = mutableListOf<ULong>()
        var consumed = 0
        var transferred = false
        try {
            if (incoming.size > MAX_OBJECTS) throw AndroidException.InvalidFile()
            incoming.forEach { file -> consumed++; ids += files.accept(file) }
            val request = prepare(ids)
            // CipherWriter first creates owning source/Lease values for every ID.
            // Retained encrypted objects may outlive this call; native Drop releases them.
            transferred = true
            return native(request)
        } finally {
            if (!transferred) ids.forEach(files::release)
            incoming.drop(consumed).forEach { try { it.close() } catch (_: Exception) {} }
        }
    }
}

/** Isolated adapter has no host paths or credentials, only one private Binder capability. */
internal class BinderDocumentPersistence(
    private val host: ICipherPersistence,
    private val files: CiphertextDescriptors,
) : DocumentPersistence {
    override fun snapshot(): CipherSnapshot = remote { snapshot(host.snapshot(), files) }
    override fun create(seed: CipherSeed) = mutation {
        transfer(seed.objects) { descriptors -> checked(host.create(metadata(seed.controls), descriptors, identifier(seed.checkpoint), identifier(seed.baseline))) }
    }
    override fun commit(commit: CipherCommit): CipherSnapshot = mutation {
        transfer(commit.objects) { descriptors ->
            if (commit.remove.size > MAX_OBJECTS) throw AndroidException.InvalidFile()
            val reply = host.commit(identifier(commit.expected), identifier(commit.control), metadata(commit.controls), descriptors,
                commit.remove.map(::identifier).toTypedArray(), identifier(commit.checkpoint), identifier(commit.baseline), metadata(commit.journal))
            commitSnapshot(reply)
        }
    }
    override fun author(authenticated: String?): ByteArray = remote {
        val result = host.author(authenticated?.let(::identifier))
        try {
            checked(result.status)
            val seed = result.seed ?: throw AndroidException.InvalidFile()
            if (seed.size != 32) throw AndroidException.InvalidFile()
            seed
        } catch (error: Exception) { result.seed?.fill(0); throw error }
    }
    override fun transportPublic(): String = remote { identifier(host.transportPublic()) }
    override fun saveDraft(file: ULong) = mutation { files.transfer(file).use { checked(host.saveDraft(it)) } }
    override fun loadDraft(): ULong? = remote {
        val result = host.loadDraft()
        try { checked(result.status); result.file?.let(files::accept) }
        catch (error: Exception) { result.file?.close(); throw error }
    }
    override fun discardDraft() = mutation { checked(host.discardDraft()) }

    @Suppress("DEPRECATION")
    private fun commitSnapshot(reply: Bundle): CipherSnapshot {
        val status = reply.get("status") as? Int
        if (status != null && status != 0) return snapshot(reply, files) // Ordinary explicit failures remain ordinary.
        return try { snapshot(reply, files) }
        catch (_: Exception) { throw AndroidException.CommitUncertain() }
    }

    private inline fun <T> transfer(ids: List<ULong>, block: (Array<ParcelFileDescriptor>) -> T): T {
        val outgoing = mutableListOf<ParcelFileDescriptor>()
        try {
            if (ids.size > MAX_OBJECTS) throw AndroidException.InvalidFile()
            ids.forEach { outgoing += files.transfer(it) }
            return block(outgoing.toTypedArray())
        } finally {
            outgoing.forEach { try { it.close() } catch (_: Exception) {} }
            ids.forEach(files::release)
        }
    }
}
