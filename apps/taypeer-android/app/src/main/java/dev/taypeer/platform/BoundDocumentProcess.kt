package dev.taypeer.platform

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Binder
import android.os.IBinder
import android.os.Looper
import android.os.ParcelFileDescriptor
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.CipherWriter
import dev.taypeer.bridge.DocumentProcess
import dev.taypeer.bridge.SelectedTransfersHost
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Fresh process generation; supervisor control only reads local state or enqueues work. */
class BoundDocumentProcess(
    context: Context,
    private val files: CiphertextDescriptors,
    private val allocate: () -> ParcelFileDescriptor,
    private val beforeStart: () -> Unit = {},
) : DocumentProcess, AutoCloseable {
    private val context = context.applicationContext
    private val callbacks = Executors.newSingleThreadExecutor()
    private val control = Executors.newSingleThreadExecutor()
    private val connected = CountDownLatch(1)
    private val started = AtomicBoolean()
    private val dead = AtomicBoolean()
    private val disposed = AtomicBoolean()
    private val terminationRequested = AtomicLong()
    private val exitObserved = AtomicLong()
    internal val terminationRequestedNanos: Long get() = terminationRequested.get()
    internal val exitObservedNanos: Long get() = exitObserved.get()
    private val owner = Binder()
    private val lifecycle = Any()
    private var starting = false
    @Volatile private var remote: IDocumentProcess? = null
    @Volatile private var streams: PipeStreams? = null
    @Volatile private var bound = false
    @Volatile var pid: Int = 0
        private set
    @Volatile internal var binder: IBinder? = null
        private set

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, service: IBinder) {
            if (disposed.get() || dead.get()) return
            try {
                service.linkToDeath({ observedExit() }, 0)
                binder = service
                remote = IDocumentProcess.Stub.asInterface(service)
            } catch (_: Exception) { observedExit() }
            finally { connected.countDown() }
        }
        override fun onServiceDisconnected(name: ComponentName) { observedExit() }
        override fun onBindingDied(name: ComponentName) { observedExit(); connected.countDown() }
        override fun onNullBinding(name: ComponentName) { observedExit(); connected.countDown() }
    }

    override fun start(writer: CipherWriter, selected: SelectedTransfersHost) {
        synchronized(lifecycle) {
            if (Looper.myLooper() == Looper.getMainLooper() || disposed.get() || dead.get() || !started.compareAndSet(false, true)) throw AndroidException.Runtime()
            starting = true
        }
        var commands: Array<ParcelFileDescriptor>? = null
        var responses: Array<ParcelFileDescriptor>? = null
        try {
            beforeStart()
            ensureLive()
            val commandPipe = ParcelFileDescriptor.createPipe().also { commands = it }
            val responsePipe = ParcelFileDescriptor.createPipe().also { responses = it }
            streams = PipeStreams(responsePipe[0], commandPipe[1])
            ensureLive()
            bound = context.bindIsolatedService(Intent(context, DocumentService::class.java), Context.BIND_AUTO_CREATE,
                "document_${UUID.randomUUID().toString().replace("-", "")}", callbacks, connection)
            ensureLive()
            if (!bound || !connected.await(10, TimeUnit.SECONDS)) throw AndroidException.Runtime()
            ensureLive()
            pid = (remote ?: throw AndroidException.Runtime()).initialize(owner, commandPipe[0], responsePipe[1],
                CipherPersistenceEndpoint(writer, files, allocate), SelectedTransfersEndpoint(selected))
            ensureLive()
        } catch (_: Exception) {
            terminate()
            throw AndroidException.Runtime()
        } finally {
            val ends = listOfNotNull(commands?.get(0), responses?.get(1),
                if (streams == null) commands?.get(1) else null)
            ends.forEach { try { it.close() } catch (_: Exception) { /* Cleanup cannot acknowledge a write. */ } }
            val finish = synchronized(lifecycle) { starting = false; disposed.get() }
            if (finish) disposeControl()
        }
    }
    private fun ensureLive() { if (disposed.get() || dead.get()) throw AndroidException.Runtime() }
    override fun read(count: UInt): ByteArray = (streams ?: throw AndroidException.Runtime()).read(count)
    override fun write(bytes: ByteArray): UInt = (streams ?: throw AndroidException.Runtime()).write(bytes)
    override fun exited(): Boolean = dead.get()
    override fun terminate() {
        if (disposed.get()) return
        terminationRequested.compareAndSet(0, System.nanoTime())
        try { control.execute { stop() } } catch (_: java.util.concurrent.RejectedExecutionException) { /* Already disposed. */ }
    }

    private fun observedExit() {
        exitObserved.compareAndSet(0, System.nanoTime())
        dead.set(true)
        terminate()
    }
    private fun stop() {
        try { remote?.terminate() } catch (_: Exception) { /* Binder death is the exit evidence. */ }
        if (bound) {
            bound = false
            try { context.unbindService(connection) } catch (_: IllegalArgumentException) { /* Already unbound. */ }
        }
        try { streams?.close() } catch (_: Exception) { /* Resource cleanup is not a durability acknowledgement. */ }
    }
    override fun close() {
        val waitForStart = synchronized(lifecycle) {
            if (!disposed.compareAndSet(false, true)) return
            starting
        }
        connected.countDown() // Cancellation never waits for the asynchronous service callback.
        if (waitForStart) control.execute { stop() } else disposeControl()
    }
    private fun disposeControl() { control.execute { stop(); callbacks.shutdown(); control.shutdown() } }
}

/** Host allocation only. The worker cannot reopen an application path or invent a fallback. */
internal fun anonymousCiphertext(context: Context): ParcelFileDescriptor {
    val directory = File(context.noBackupFilesDir, "ciphertext-temporary")
    if (!directory.isDirectory && !directory.mkdirs()) throw AndroidException.StorageIo()
    val file = File.createTempFile("cipher-", ".tmp", directory)
    try {
        val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
        if (!file.delete()) { descriptor.close(); throw AndroidException.StorageIo() }
        return descriptor
    } catch (_: Exception) {
        file.delete()
        throw AndroidException.StorageIo()
    }
}
