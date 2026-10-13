package dev.taypeer.platform

import android.os.ParcelFileDescriptor
import java.io.IOException
import java.util.concurrent.atomic.AtomicLong
import javax.crypto.Cipher

/** Private OS adapter only. Plaintext stays in native scratch and the worker's key fd. */
internal class BiometricKeyPipe private constructor(handle: Long) : AutoCloseable {
    private val handle = AtomicLong(handle)

    /** FD publication is an intermediate result; the worker must authenticate the current archive. */
    fun finish(cipher: Cipher, wrapped: ByteArray): Boolean {
        val selected = handle.get()
        if (selected == 0L) return false
        try { return BiometricKeyPipeNative.finish(selected, cipher, wrapped) }
        finally {
            // Keep the handle reachable while provider I/O runs: close() must
            // be able to cancel without waiting for it or reusing an fd number.
            handle.compareAndSet(selected, 0L)
            BiometricKeyPipeNative.cancel(selected)
        }
    }

    override fun close() {
        handle.getAndSet(0L).takeIf { it != 0L }?.let(BiometricKeyPipeNative::cancel)
    }

    companion object {
        /** Consumes a writable pipe. The native attempt is its only remaining writer. */
        fun open(output: ParcelFileDescriptor): BiometricKeyPipe {
            var handle = 0L
            try {
                output.use {
                    handle = BiometricKeyPipeNative.create(it.fd)
                    if (handle == 0L) throw IOException("Biometric key pipe unavailable")
                }
                return BiometricKeyPipe(handle)
            } catch (failure: Exception) {
                if (handle != 0L) BiometricKeyPipeNative.cancel(handle)
                throw failure
            }
        }
    }
}

/** No direct buffer or ReadKey getter is part of this JNI interface. */
internal object BiometricKeyPipeNative {
    init { System.loadLibrary("taypeer_biometric_pipe") }
    @JvmStatic external fun create(outputFd: Int): Long
    @JvmStatic external fun finish(handle: Long, cipher: Cipher, wrapped: ByteArray): Boolean
    @JvmStatic external fun cancel(handle: Long)
}
