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
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** Application-owned sessions survive Activity recreation; no form values are persisted here. */
class DocumentManager(
    context: Context,
    private val host: Host,
    private val allocate: () -> ParcelFileDescriptor = { anonymousCiphertext(context) },
) : AutoCloseable {
    private val context = context.applicationContext
    private data class Owned(val session: DocumentSession, val process: BoundDocumentProcess, val files: CiphertextDescriptors)
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
    fun open(path: String, password: String): DocumentSession = start { process, files ->
        host.openDocument(path, password, process, files)
    }
    fun import(uri: Uri, password: String): DocumentSession {
        offUi()
        val copy = WorkingCopies(context, host).import(uri)
        return open(copy.path, password)
    }
    fun catalog(): List<WorkingCopyView> { offUi(); return host.workingCopies() }
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
        val owned = sessions[session.generation()] ?: return
        session.closeSession()
        sessions.remove(session.generation(), owned)
        dispose(owned)
    }
    override fun close() {
        offUi()
        background()
        var failure: Exception? = null
        sessions.values.toList().forEach { owned ->
            try { owned.session.closeSession() } catch (error: Exception) { if (failure == null) failure = error }
            finally { sessions.remove(owned.session.generation(), owned); dispose(owned) }
        }
        failure?.let { throw it }
    }
    internal fun process(session: DocumentSession): BoundDocumentProcess = sessions[session.generation()]?.process ?: throw AndroidException.Runtime()

    private fun start(open: (BoundDocumentProcess, CiphertextDescriptors) -> DocumentSession): DocumentSession {
        offUi()
        val startedAt = epoch.get()
        val files = CiphertextDescriptors(allocate)
        val process = BoundDocumentProcess(context, files, allocate) { beforeProcessStart?.invoke() }
        pending[process] = startedAt
        try {
            checkEpoch(startedAt)
            val session = open(process, files)
            val owned = Owned(session, process, files)
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
