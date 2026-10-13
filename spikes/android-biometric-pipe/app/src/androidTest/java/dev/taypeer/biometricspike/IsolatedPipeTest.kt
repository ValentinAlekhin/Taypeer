package dev.taypeer.biometricspike

import android.os.ParcelFileDescriptor
import android.system.Os
import android.system.OsConstants
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.io.FileDescriptor
import org.junit.Assert.*
import org.junit.Test

/** Actual isolated UID, Binder descriptor grant and Native validation of fixed PUBLIC fixtures. */
class IsolatedPipeTest {
    private fun publicInput(bytes: ByteArray, expectedExact: Boolean, expectedFailure: String = "none") {
        val descriptors = ParcelFileDescriptor.createPipe()
        try {
            // At most 33 PUBLIC bytes fit without a reader; close the writer to establish exact EOF.
            ParcelFileDescriptor.AutoCloseOutputStream(descriptors[1]).use { it.write(bytes) }
            PublicIsolatedNativeProbe.verify(descriptors[0], expectedExact, expectedFailure)
        } finally {
            descriptors[0].close()
            descriptors[1].close()
        }
    }

    @Test(timeout = 20_000) fun isolatedNativeDuplicatesTheHeldFifoAndAcceptsExactPublicBytesWithEof() {
        publicInput(ByteArray(32) { 0x50 }, expectedExact = true)
    }

    @Test(timeout = 40_000) fun isolatedNativeRefusesShortExtraAndDifferentPublicBytesWithoutClosingOriginal() {
        publicInput(ByteArray(31) { 0x50 }, expectedExact = false, expectedFailure = "InvalidFile")
        publicInput(ByteArray(33) { 0x50 }, expectedExact = false, expectedFailure = "InvalidFile")
        publicInput(ByteArray(32) { 0x51 }, expectedExact = false)
    }

    @Test(timeout = 20_000) fun isolatedNativeRefusesRegularFileWithoutReadingOrClosingOriginal() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val file = File.createTempFile("PUBLIC-isolated-input", ".fixture", context.cacheDir)
        val bytes = ByteArray(32) { 0x50 }
        try {
            file.writeBytes(bytes)
            ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use { original ->
                PublicIsolatedNativeProbe.verify(original, expectedExact = false, expectedFailure = "InvalidFile")
                assertEquals("The rejected file must not have been read", 0L, Os.lseek(original.fileDescriptor, 0, OsConstants.SEEK_CUR))
            }
            assertArrayEquals(bytes, file.readBytes())
        } finally {
            assertTrue(file.delete())
        }
    }

    @Test(timeout = 20_000) fun isolatedNativeRefusesSocketBeforeReadWhileBothPeersRemainOpen() {
        val left = FileDescriptor()
        val right = FileDescriptor()
        Os.socketpair(OsConstants.AF_UNIX, OsConstants.SOCK_STREAM, 0, left, right)
        try {
            // No socket data or EOF is supplied: a pre-read descriptor refusal must return.
            ParcelFileDescriptor.dup(left).use { original ->
                PublicIsolatedNativeProbe.verify(original, expectedExact = false, expectedFailure = "InvalidFile")
            }
            Os.fstat(left)
            Os.fstat(right)
        } finally {
            try { Os.close(left) } finally { Os.close(right) }
        }
    }

    @Test(timeout = 20_000) fun isolatedNativeRefusesWriterBeforeReadAndPreservesBothPipeEnds() {
        val descriptors = ParcelFileDescriptor.createPipe()
        try {
            // Both ends stay open and no bytes are supplied; the write-only FD must be rejected.
            PublicIsolatedNativeProbe.verify(descriptors[1], expectedExact = false, expectedFailure = "InvalidFile")
            Os.fstat(descriptors[0].fileDescriptor)
        } finally {
            descriptors[0].close()
            descriptors[1].close()
        }
    }

    @Test(timeout = 20_000) fun isolatedNativeRefusesNonblockingReaderAndPreservesItsFlagsAndOriginal() {
        val descriptors = ParcelFileDescriptor.createPipe()
        try {
            val original = descriptors[0]
            val flags = Os.fcntlInt(original.fileDescriptor, OsConstants.F_GETFL, 0)
            Os.fcntlInt(original.fileDescriptor, OsConstants.F_SETFL, flags or OsConstants.O_NONBLOCK)
            // The writer stays open with no bytes: this must be a flag refusal, never a key read.
            PublicIsolatedNativeProbe.verify(original, expectedExact = false, expectedFailure = "InvalidFile")
            assertEquals(flags or OsConstants.O_NONBLOCK, Os.fcntlInt(original.fileDescriptor, OsConstants.F_GETFL, 0))
            Os.fstat(descriptors[1].fileDescriptor)
        } finally {
            descriptors[0].close()
            descriptors[1].close()
        }
    }
}
