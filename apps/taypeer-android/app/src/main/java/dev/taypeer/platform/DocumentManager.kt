package dev.taypeer.platform

import android.content.Context
import android.net.Uri
import android.os.Looper
import android.os.ParcelFileDescriptor
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.EnrollmentProgress
import dev.taypeer.bridge.EnrollmentRow
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.WorkingCopyView
import dev.taypeer.bridge.PendingRecoveryView
import dev.taypeer.bridge.SessionAccess
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** A real registered copy and its opaque OPEN worker; no document or key is cached here. */
internal class OpenDocumentSource(val copy: WorkingCopyView, val session: DocumentSession) {
    val generation: ULong = session.generation()
    fun permitted(): Boolean = try { session.generation() == generation && session.access() == SessionAccess.OPEN }
        catch (_: IllegalStateException) { false } // An already disposed opaque handle is never an OPEN source.
}

/** Application-owned sessions survive Activity recreation; no form values are persisted here. */
class DocumentManager(
    context: Context,
    private val host: Host,
    private val allocate: () -> ParcelFileDescriptor = { anonymousCiphertext(context) },
) : AutoCloseable {
    private val context = context.applicationContext
    private data class Owned(val session: DocumentSession, val process: BoundDocumentProcess, val files: CiphertextDescriptors,
        val database: String, val source: OpenDocumentSource?)
    private val sessions = ConcurrentHashMap<ULong, Owned>()
    private val pending = ConcurrentHashMap<BoundDocumentProcess, Long>()
    private val epoch = AtomicLong()
    @Volatile var clearSensitiveUi: (() -> Unit)? = null
    @Volatile internal var beforeProcessStart: (() -> Unit)? = null
    internal val pendingLaunchCount: Int get() = pending.size

    /** Every operation that authenticates, writes or waits for an OS process runs off the UI thread. */
    fun create(name: String, description: String?, operation: String, password: String): DocumentSession = start { process, files ->
        host.createDocument(name, description, operation, password, process, files)
    }
    fun open(path: String, password: String): DocumentSession {
        offUi()
        return unlockedSources().firstOrNull { it.copy.path == path }?.session ?: start { process, files ->
            host.openDocument(path, password, process, files)
        }
    }
    /** Native pending worker authenticates the exact current key before returning OPEN. */
    fun openBiometric(copy: WorkingCopyView): DocumentSession {
        offUi()
        return unlockedSource(copy)?.session ?: start { process, files ->
            host.prepareBiometricOpen(copy, process, files).use { pending -> pending.complete() }
        }
    }
    /** Constrained source reader; no old-chain exchange or normal document-writing capability. */
    fun openRecovery(copy: WorkingCopyView, password: String, operation: String): DocumentSession = start(searchable = false) { process, files ->
        host.openRecoveryDocument(copy.path, password, operation, process, files)
    }
    fun pendingRecoveries(): List<PendingRecoveryView> { offUi(); return host.pendingRecoveries() }
    fun closeRecoverySource(database: String) {
        offUi()
        sessions.values.toList().filter { it.database == database }.forEach { owned -> close(owned.session) }
    }
    /** Actual closeSession/OS exit and released Binder writers precede protected catalog adoption. */
    fun adoptRecovery(publication: ByteArray, database: String): WorkingCopyView {
        offUi()
        val startedAt = epoch.get()
        closeRecoverySource(database)
        checkEpoch(startedAt)
        return host.adoptRecovery(publication).also { checkEpoch(startedAt) }
    }
    fun import(uri: Uri, password: String): DocumentSession {
        offUi()
        val copy = WorkingCopies(context, host).import(uri)
        return open(copy.path, password)
    }
    fun catalog(): List<WorkingCopyView> { offUi(); return host.workingCopies() }
    /** Fast control checks only. Revoked workers are never search sources or activation candidates. */
    internal fun unlockedSources(): List<OpenDocumentSource> = sessions.values.mapNotNull { it.source?.takeIf(OpenDocumentSource::permitted) }
    internal fun unlockedSource(copy: WorkingCopyView): OpenDocumentSource? = unlockedSources()
        .firstOrNull { it.copy.path == copy.path && it.copy.database == copy.database }
    internal fun unlockedSource(session: DocumentSession): OpenDocumentSource? = unlockedSources().firstOrNull { it.session === session }
    internal fun closeRevoked() {
        offUi()
        sessions.values.toList().filter {
            try { it.session.access() != SessionAccess.OPEN } catch (_: IllegalStateException) { true }
        }.forEach { close(it.session) }
    }
    /** A fresh proof-only worker closes before the host starts network enrollment. */
    fun join(code: String): EnrollmentProgress {
        offUi()
        val startedAt = epoch.get()
        val files = CiphertextDescriptors(allocate)
        val process = BoundDocumentProcess(context, files, allocate) { beforeProcessStart?.invoke() }
        pending[process] = startedAt
        try {
            checkEpoch(startedAt)
            val progress = host.joinInvitation(code, process, files)
            checkEpoch(startedAt)
            return progress
        } finally { pending.remove(process); process.close(); files.close() }
    }
    fun resumeJoin(request: String): EnrollmentProgress { offUi(); return host.resumeJoin(request) }
    fun pendingJoins(): List<EnrollmentRow> { offUi(); return host.pendingJoins() }
    fun activity() { host.activity() }

    /** UI secrecy and access revocation never wait for a blocked command or filesystem. */
    fun background() {
        epoch.incrementAndGet()
        pending.keys.toList().forEach { it.close() }
        try { clearSensitiveUi?.invoke() } finally { host.background() }
    }
    fun lock(session: DocumentSession) { session.lock() }
    fun close(session: DocumentSession) {
        offUi()
        val owned = sessions.values.firstOrNull { it.session === session } ?: return
        synchronized(owned) {
            val generation = sessions.entries.firstOrNull { it.value === owned }?.key ?: return
            session.closeSession()
            sessions.remove(generation, owned)
            dispose(owned)
        }
    }
    override fun close() {
        offUi()
        background()
        var failure: Exception? = null
        sessions.values.toList().forEach { owned ->
            synchronized(owned) {
                val generation = sessions.entries.firstOrNull { it.value === owned }?.key
                if (generation != null) {
                    try { owned.session.closeSession() } catch (error: Exception) { if (failure == null) failure = error }
                    finally { sessions.remove(generation, owned); dispose(owned) }
                }
            }
        }
        failure?.let { throw it }
    }
    internal fun process(session: DocumentSession): BoundDocumentProcess = sessions[session.generation()]?.process ?: throw AndroidException.Runtime()

    private fun start(searchable: Boolean = true, open: (BoundDocumentProcess, CiphertextDescriptors) -> DocumentSession): DocumentSession {
        offUi()
        val startedAt = epoch.get()
        val files = CiphertextDescriptors(allocate)
        val process = BoundDocumentProcess(context, files, allocate) { beforeProcessStart?.invoke() }
        pending[process] = startedAt
        try {
            checkEpoch(startedAt)
            val session = open(process, files)
            val database = session.databaseId()
            val source = if (searchable) try {
                OpenDocumentSource(host.workingCopies().single { it.database == database }, session)
            } catch (failure: Exception) {
                session.lock()
                try { session.closeSession() } finally { session.close() }
                throw failure
            } else null
            val owned = Owned(session, process, files, database, source)
            if (epoch.get() != startedAt || sessions.putIfAbsent(session.generation(), owned) != null) {
                session.lock()
                try { session.closeSession() } finally { session.close() }
                throw AndroidException.Runtime()
            }
            return session
        } catch (error: Exception) {
            process.close()
            files.close()
            throw error
        } finally { pending.remove(process) }
    }
    private fun checkEpoch(startedAt: Long) { if (epoch.get() != startedAt) throw AndroidException.Runtime() }
    private fun dispose(owned: Owned) {
        owned.process.close()
        owned.files.close()
        owned.session.close() // Dispose the UniFFI reference after confirmed native closeSession().
    }
    private fun offUi() {
        if (Looper.myLooper() == Looper.getMainLooper()) throw AndroidException.Runtime()
    }
}
