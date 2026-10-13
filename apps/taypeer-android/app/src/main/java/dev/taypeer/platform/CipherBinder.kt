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
import dev.taypeer.bridge.CipherRecoveryArchive
import dev.taypeer.bridge.CipherRecoverySource
import dev.taypeer.bridge.DocumentPersistence

private const val MAX_METADATA = 1024 * 1024
private const val MAX_OBJECTS = 4096
private const val MAX_RECOVERY_METADATA = 64 * 1024

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
private fun recoveryMetadata(bytes: ByteArray): ByteArray {
    if (bytes.size > MAX_RECOVERY_METADATA) throw AndroidException.InvalidFile()
    return bytes
}
private fun recoveryBundle(view: CipherRecoveryArchive?, transfer: (ULong) -> ParcelFileDescriptor): Bundle = Bundle().apply {
    putInt("status", 0)
    putBoolean("present", view != null)
    if (view != null) {
        putParcelable("file", transfer(view.file))
        putString("root", view.root)
        putString("fingerprint", view.fingerprint)
    }
}
private fun recoveryBundle(view: CipherRecoverySource?, transfer: (ULong) -> ParcelFileDescriptor): Bundle =
    recoveryBundle(view?.archive, transfer).apply {
        if (view != null) {
            putByteArray("source", recoveryMetadata(view.source))
            putBundle("candidate", recoveryBundle(view.candidate, transfer))
            putByteArray("intent", view.intent?.let(::recoveryMetadata))
        }
    }

@Suppress("DEPRECATION")
private fun recoveryArchive(bundle: Bundle, files: CiphertextDescriptors, source: Boolean = false): CipherRecoveryArchive? {
    val file = if (Build.VERSION.SDK_INT >= 33) bundle.getParcelable("file", ParcelFileDescriptor::class.java)
        else bundle.getParcelable<ParcelFileDescriptor>("file")
    try {
        checked(bundle.getInt("status", 6))
        val present = bundle.get("present") as? Boolean ?: throw AndroidException.InvalidFile()
        if (!present) {
            if (bundle.keySet() != setOf("status", "present") || file != null) throw AndroidException.InvalidFile()
            return null
        }
        val keys = setOf("status", "present", "file", "root", "fingerprint") + if (source) setOf("source", "candidate", "intent") else emptySet()
        if (bundle.keySet() != keys || file == null) throw AndroidException.InvalidFile()
        val root = identifier(bundle.getString("root") ?: throw AndroidException.InvalidFile())
        val fingerprint = identifier(bundle.getString("fingerprint") ?: throw AndroidException.InvalidFile())
        return CipherRecoveryArchive(files.accept(file), root, fingerprint)
    } catch (error: Exception) { file?.close(); throw error }
}
private fun recoverySource(bundle: Bundle, files: CiphertextDescriptors): CipherRecoverySource? {
    var archive: CipherRecoveryArchive? = null
    var candidate: CipherRecoveryArchive? = null
    val candidateBundle = bundle.getBundle("candidate")
    try {
        archive = recoveryArchive(bundle, files, source = true) ?: return null
        candidate = recoveryArchive(candidateBundle ?: throw AndroidException.InvalidFile(), files)
        val source = recoveryMetadata(bundle.getByteArray("source") ?: throw AndroidException.InvalidFile())
        val intent = bundle.getByteArray("intent")?.let(::recoveryMetadata)
        if ((candidate == null) != (intent == null) || source.size + (intent?.size ?: 0) > MAX_RECOVERY_METADATA) throw AndroidException.InvalidFile()
        return CipherRecoverySource(source, archive, candidate, intent)
    } catch (error: Exception) {
        archive?.let { files.release(it.file) }; candidate?.let { files.release(it.file) }
        closeRecoveryFile(bundle); candidateBundle?.let(::closeRecoveryFile)
        throw error
    }
}
@Suppress("DEPRECATION")
private fun closeRecoveryFile(bundle: Bundle) {
    val descriptor = if (Build.VERSION.SDK_INT >= 33) bundle.getParcelable("file", ParcelFileDescriptor::class.java)
        else bundle.getParcelable<ParcelFileDescriptor>("file")
    try { descriptor?.close() } catch (_: Exception) { /* Cleanup never acknowledges publication. */ }
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
    writer: CipherWriter,
    private val files: CiphertextDescriptors,
    private val allocate: () -> ParcelFileDescriptor,
) : ICipherPersistence.Stub(), AutoCloseable {
    private val lifetime = Any()
    private var writer: CipherWriter? = writer
    private var accepting = true
    private var calls = 0
    private val outgoing = ThreadLocal<MutableList<ParcelFileDescriptor>>()

    // Binder may retain this endpoint after its worker has exited. Revoke new calls
    // immediately, but let each accepted call finish before releasing the native Arc.
    override fun close() {
        val release = synchronized(lifetime) {
            accepting = false
            if (calls == 0) writer.also { writer = null } else null
        }
        release?.close()
    }
    private inline fun <T> withWriter(block: (CipherWriter) -> T): T {
        val active = synchronized(lifetime) {
            if (!accepting) throw AndroidException.Runtime()
            val active = writer ?: throw AndroidException.Runtime()
            calls++
            active
        }
        try { return block(active) }
        finally {
            val release = synchronized(lifetime) {
                calls--
                if (!accepting && calls == 0) writer.also { writer = null } else null
            }
            release?.close()
        }
    }
    private inline fun <T> withIncoming(incoming: Array<ParcelFileDescriptor>, block: (CipherWriter) -> T): T {
        var accepted = false
        try {
            return withWriter { active ->
                accepted = true
                block(active)
            }
        } finally {
            // A rejected late call still owns the descriptors received through Binder.
            if (!accepted) incoming.forEach { try { it.close() } catch (_: Exception) {} }
        }
    }
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
    override fun createTemporary(): CipherFileResult = try { withWriter { CipherFileResult(0, send(allocate())) } } catch (error: Exception) { CipherFileResult(code(error), null) }
    override fun snapshot(): Bundle = snapshotResult { withWriter { snapshotBundle(it.snapshot(), ::transfer) } }
    override fun create(controls: ByteArray, objects: Array<ParcelFileDescriptor>, checkpoint: String, baseline: String): Int = status {
        withIncoming(objects) { active ->
            consume(objects, { ids -> CipherSeed(metadata(controls), ids, identifier(checkpoint), identifier(baseline)) }, active::create)
        }
    }
    override fun commit(expected: String, control: String, controls: ByteArray, objects: Array<ParcelFileDescriptor>, remove: Array<String>, checkpoint: String, baseline: String, journal: ByteArray): Bundle = snapshotResult {
        withIncoming(objects) { active ->
            consume(objects, { ids ->
                if (remove.size > MAX_OBJECTS) throw AndroidException.InvalidFile()
                CipherCommit(identifier(expected), identifier(control), metadata(controls), ids,
                    remove.map(::identifier), identifier(checkpoint), identifier(baseline), metadata(journal))
            }, { request ->
                val committed = active.commit(request) // Preserve explicit native errors before success.
                try { snapshotBundle(committed, ::transfer) }
                catch (_: Exception) { throw AndroidException.CommitUncertain() }
            })
        }
    }
    override fun author(authenticated: String?): CipherAuthorResult = try { withWriter { CipherAuthorResult(0, it.author(authenticated?.let(::identifier))) } } catch (error: Exception) { CipherAuthorResult(code(error), null) }
    override fun transportPublic(): String = remote { withWriter { it.transportPublic() } }
    override fun saveDraft(file: ParcelFileDescriptor): Int = status {
        val incoming = arrayOf(file)
        withIncoming(incoming) { active ->
            consume(incoming, { ids -> ids.single() }, active::saveDraft)
        }
    }
    override fun loadDraft(): CipherFileResult = try { withWriter { CipherFileResult(0, it.loadDraft()?.let(::transfer)) } } catch (error: Exception) { CipherFileResult(code(error), null) }
    override fun discardDraft(): Int = status { withWriter { it.discardDraft() } }
    override fun freezeRecovery(operation: String, authenticated: String): Bundle = snapshotResult {
        withWriter { recoveryBundle(it.freezeRecovery(identifier(operation), identifier(authenticated)), ::transfer) }
    }
    override fun recoverySource(operation: String): Bundle = snapshotResult {
        withWriter { recoveryBundle(it.recoverySource(identifier(operation)), ::transfer) }
    }
    override fun authorRecovery(source: ByteArray): CipherAuthorResult = try {
        withWriter { CipherAuthorResult(0, it.authorRecovery(recoveryMetadata(source))) }
    } catch (error: Exception) { CipherAuthorResult(code(error), null) }
    override fun recoveryCandidate(intent: ByteArray): Bundle = snapshotResult {
        withWriter { recoveryBundle(it.recoveryCandidate(recoveryMetadata(intent)), ::transfer) }
    }
    override fun publishRecovery(intent: ByteArray, controls: ByteArray, objects: Array<ParcelFileDescriptor>, checkpoint: String, baseline: String): Bundle = snapshotResult {
        withIncoming(objects) { active ->
            consume(objects, { ids -> recoveryMetadata(intent) to CipherSeed(metadata(controls), ids, identifier(checkpoint), identifier(baseline)) }, { (publicIntent, seed) ->
                val result = active.publishRecovery(publicIntent, seed)
                try { Bundle().apply { putInt("status", 0); putByteArray("publication", recoveryMetadata(result)) } }
                catch (_: Exception) { throw AndroidException.CommitUncertain() }
            })
        }
    }
    override fun confirmRecovery(publication: ByteArray): Int = status {
        withWriter { it.confirmRecovery(recoveryMetadata(publication)) }
    }

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
    override fun freezeRecovery(operation: String, authenticated: String): CipherRecoverySource = mutation {
        recoverySource(host.freezeRecovery(identifier(operation), identifier(authenticated)), files) ?: throw AndroidException.InvalidFile()
    }
    override fun recoverySource(operation: String): CipherRecoverySource? = remote {
        recoverySource(host.recoverySource(identifier(operation)), files)
    }
    override fun authorRecovery(source: ByteArray): ByteArray = remote {
        val result = host.authorRecovery(recoveryMetadata(source))
        try {
            checked(result.status)
            val seed = result.seed ?: throw AndroidException.InvalidFile()
            if (seed.size != 32) throw AndroidException.InvalidFile()
            seed
        } catch (error: Exception) { result.seed?.fill(0); throw error }
    }
    override fun recoveryCandidate(intent: ByteArray): CipherRecoveryArchive? = remote {
        recoveryArchive(host.recoveryCandidate(recoveryMetadata(intent)), files)
    }
    override fun publishRecovery(intent: ByteArray, seed: CipherSeed): ByteArray = mutation {
        transfer(seed.objects) { descriptors ->
            val result = host.publishRecovery(recoveryMetadata(intent), metadata(seed.controls), descriptors,
                identifier(seed.checkpoint), identifier(seed.baseline))
            checked(result.getInt("status", 6))
            try {
                if (result.keySet() != setOf("status", "publication")) throw AndroidException.InvalidFile()
                recoveryMetadata(result.getByteArray("publication") ?: throw AndroidException.InvalidFile())
            } catch (_: Exception) { throw AndroidException.CommitUncertain() }
        }
    }
    override fun confirmRecovery(publication: ByteArray) = mutation {
        checked(host.confirmRecovery(recoveryMetadata(publication)))
    }

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
