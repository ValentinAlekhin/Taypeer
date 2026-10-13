package dev.taypeer

import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.platform.SelectedDescriptorOutput
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Actual descriptors exercise the completion boundary before any Native success reply. */
@RunWith(AndroidJUnit4::class)
class SelectedOutputCompletionTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test fun finishClosesTheOriginalRegularDestinationBeforeAcknowledging() {
        val file = destination()
        val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
        val output = SelectedDescriptorOutput(descriptor)
        val public = "PUBLIC selected output completion".toByteArray()
        try {
            assertEquals(public.size.toUInt(), output.write(0uL, public.copyOf()))
            assertTrue(descriptor.fileDescriptor.valid())
            output.finish()
            assertFalse(descriptor.fileDescriptor.valid())
            assertArrayEquals(public, file.readBytes())
            output.close(); output.close()
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
            val late = byteArrayOf(42)
            assertThrows(AndroidException.InvalidOptions::class.java) { output.write(0uL, late) }
            assertArrayEquals(byteArrayOf(0), late)
            assertArrayEquals(public, file.readBytes())
        } finally { output.close(); assertTrue(file.delete()) }
    }

    @Test fun anOriginalCloseFailureRejectsCompletionEvenAfterActualFsyncAndClose() {
        val file = destination()
        val original = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
        var closes = 0
        // The public PFD wrapper preserves the real write/fsync path; only its
        // final close reports the provider failure that dup-only finish missed.
        val descriptor = object : ParcelFileDescriptor(original) {
            override fun close() {
                closes++
                super.close()
                throw IOException("PUBLIC selected destination close failure")
            }
        }
        val output = SelectedDescriptorOutput(descriptor)
        val public = "PUBLIC bytes are not a completion receipt".toByteArray()
        try {
            assertEquals(public.size.toUInt(), output.write(0uL, public.copyOf()))
            assertThrows(AndroidException.StorageIo::class.java) { output.finish() }
            assertEquals(1, closes)
            assertFalse(original.fileDescriptor.valid())
            assertArrayEquals(public, file.readBytes())
            output.close(); output.close()
            assertEquals(1, closes)
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
        } finally { output.close(); original.close(); assertTrue(file.delete()) }
    }

    @Test fun aReliablePipeProviderFailureCannotAcknowledgeAFileExport() {
        val pipe = ParcelFileDescriptor.createReliablePipe()
        val output = SelectedDescriptorOutput(pipe[1])
        try {
            assertTrue(pipe[1].canDetectErrors())
            pipe[0].closeWithError("PUBLIC selected provider rejected output")
            assertThrows(AndroidException.StorageIo::class.java) { output.finish() }
            assertFalse(pipe[1].fileDescriptor.valid())
            output.close(); output.close()
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
        } finally { output.close(); pipe.forEach { it.close() } }
    }

    @Test fun aReliableProviderErrorCollectedDuringCloseRejectsCompletion() {
        val file = destination()
        val original = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
        val pipe = ParcelFileDescriptor.createReliablePipe()
        // Real file I/O and a real reliable error channel reproduce a provider
        // status arriving at close, after the preceding fsync/error check.
        val descriptor = object : ParcelFileDescriptor(original) {
            override fun canDetectErrors() = pipe[1].canDetectErrors()
            override fun checkError() = pipe[1].checkError()
            override fun close() {
                pipe[0].closeWithError("PUBLIC selected provider failed during close")
                try { super.close() } finally { pipe[1].close() }
            }
        }
        val output = SelectedDescriptorOutput(descriptor)
        val public = "PUBLIC late provider completion failure".toByteArray()
        try {
            assertEquals(public.size.toUInt(), output.write(0uL, public.copyOf()))
            assertThrows(AndroidException.StorageIo::class.java) { output.finish() }
            assertFalse(original.fileDescriptor.valid())
            assertFalse(pipe[1].fileDescriptor.valid())
            assertArrayEquals(public, file.readBytes())
            output.close(); output.close()
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
        } finally {
            output.close(); original.close(); pipe.forEach { it.close() }
            assertTrue(file.delete())
        }
    }

    @Test fun aProviderCloseCompletesOutsideTheOwnershipLockBeforeTheReply() {
        val file = destination()
        val original = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
        val closing = CountDownLatch(1)
        val release = CountDownLatch(1)
        val descriptor = object : ParcelFileDescriptor(original) {
            override fun close() {
                closing.countDown()
                try { check(release.await(5, TimeUnit.SECONDS)) }
                finally { super.close() }
            }
        }
        val output = SelectedDescriptorOutput(descriptor)
        val executor = Executors.newFixedThreadPool(2)
        try {
            val completed = executor.submit { output.finish() }
            assertTrue(closing.await(5, TimeUnit.SECONDS))
            assertFalse(completed.isDone)
            assertTrue(original.fileDescriptor.valid())
            // Cleanup must not wait on provider I/O under the descriptor lock.
            executor.submit { output.close() }.get(5, TimeUnit.SECONDS)
            assertFalse(completed.isDone)
            release.countDown()
            completed.get(5, TimeUnit.SECONDS)
            assertFalse(original.fileDescriptor.valid())
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
        } finally {
            release.countDown()
            executor.shutdown()
            assertTrue(executor.awaitTermination(5, TimeUnit.SECONDS))
            output.close(); original.close(); assertTrue(file.delete())
        }
    }

    @Test fun anAlreadyClosedDestinationFailsAndWipesItsWriteChunk() {
        val file = destination()
        val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
        val output = SelectedDescriptorOutput(descriptor)
        val public = "PUBLIC original selected bytes".toByteArray()
        try {
            assertEquals(public.size.toUInt(), output.write(0uL, public.copyOf()))
            descriptor.close()
            val late = byteArrayOf(42)
            assertThrows(AndroidException.StorageIo::class.java) { output.write(0uL, late) }
            assertArrayEquals(byteArrayOf(0), late)
            assertThrows(AndroidException.StorageIo::class.java) { output.finish() }
            output.close(); output.close()
            assertFalse(descriptor.fileDescriptor.valid())
            assertArrayEquals(public, file.readBytes())
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
        } finally { output.close(); assertTrue(file.delete()) }
    }

    private fun destination(): File = File(context.cacheDir, "PUBLIC-output-${UUID.randomUUID()}.bin").apply {
        writeBytes(byteArrayOf())
    }
}
