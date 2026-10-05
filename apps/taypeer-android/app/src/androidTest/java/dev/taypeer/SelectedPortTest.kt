package dev.taypeer

import android.net.Uri
import android.os.Parcel
import android.os.ParcelFileDescriptor
import android.os.Parcelable
import android.os.RemoteException
import android.system.Os
import android.system.OsConstants
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.platform.BinderSelectedTransfers
import dev.taypeer.platform.ISelectedTransfers
import dev.taypeer.platform.SelectedChunk
import dev.taypeer.platform.SelectedDescriptorInput
import dev.taypeer.platform.SelectedDescriptorOutput
import dev.taypeer.platform.SelectedDescriptors
import dev.taypeer.platform.SelectedWriteResult
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Contract checks complement, rather than substitute for, actual document-worker transfers. */
@RunWith(AndroidJUnit4::class)
class SelectedPortTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private open class Port : ISelectedTransfers.Stub() {
        override fun read(id: Long, offset: Long, count: Int) = SelectedChunk(1, null)
        override fun write(id: Long, offset: Long, bytes: ByteArray) = SelectedWriteResult(1, 0)
        override fun finish(id: Long) = 1
    }
    @Test fun repliesAreBoundedAndLostWriteOrFinishCannotAcknowledgeDurability() {
        val explicit = BinderSelectedTransfers(Port())
        assertThrows(AndroidException.StorageIo::class.java) { explicit.finish(1uL) }
        val lost = BinderSelectedTransfers(object : Port() {
            override fun read(id: Long, offset: Long, count: Int): SelectedChunk = throw RemoteException()
            override fun write(id: Long, offset: Long, bytes: ByteArray): SelectedWriteResult = throw RemoteException()
            override fun finish(id: Long): Int = throw RemoteException()
        })
        assertThrows(AndroidException.Runtime::class.java) { lost.read(1uL, 0uL, 1u) }
        val sent = byteArrayOf(42)
        assertThrows(AndroidException.CommitUncertain::class.java) { lost.write(1uL, 0uL, sent) }
        assertArrayEquals(byteArrayOf(0), sent)
        assertThrows(AndroidException.CommitUncertain::class.java) { lost.finish(1uL) }
        val unexpected = byteArrayOf(42, 43)
        val oversized = BinderSelectedTransfers(object : Port() {
            override fun read(id: Long, offset: Long, count: Int) = SelectedChunk(0, unexpected)
        })
        assertThrows(AndroidException.InvalidFile::class.java) { oversized.read(1uL, 0uL, 1u) }
        assertArrayEquals(byteArrayOf(0, 0), unexpected)
        assertThrows(AndroidException.InvalidOptions::class.java) { explicit.read(1uL, 0uL, 65537u) }
    }
    @Test fun parcelSerializationClearsSenderWhileReceiverGetsOnlyTheBoundedChunk() {
        val bytes = byteArrayOf(41, 42, 43)
        val parcel = Parcel.obtain()
        try {
            SelectedChunk(0, bytes).writeToParcel(parcel, Parcelable.PARCELABLE_WRITE_RETURN_VALUE)
            assertArrayEquals(byteArrayOf(0, 0, 0), bytes)
            parcel.setDataPosition(0)
            val received = SelectedChunk.CREATOR.createFromParcel(parcel)
            assertArrayEquals(byteArrayOf(41, 42, 43), received.bytes)
            received.bytes?.fill(0)
        } finally { parcel.recycle() }
    }
    @Test fun selectedDescriptorsUseIndependentOffsetsAndRejectUnknownPipeWithoutCopying() {
        assertThrows(AndroidException.StorageIo::class.java) {
            SelectedDescriptors.openInput(context, Uri.parse("content://dev.taypeer.publicselected/PUBLIC-unknown"))
        }
        val pipe = ParcelFileDescriptor.createPipe()
        try {
            assertThrows(AndroidException.StorageIo::class.java) { SelectedDescriptorInput(pipe[0]) }
            assertFalse(pipe[0].fileDescriptor.valid())
        } finally { pipe.forEach { it.close() } }
        val file = File(context.cacheDir, "PUBLIC-selected-${UUID.randomUUID()}.bin")
        file.writeBytes(byteArrayOf(11, 12, 13, 14))
        try {
            val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
            Os.lseek(descriptor.fileDescriptor, 1, OsConstants.SEEK_SET)
            val input = SelectedDescriptorInput(descriptor)
            assertEquals(4uL, input.length())
            assertArrayEquals(byteArrayOf(13, 14), input.read(2uL, 2u))
            assertArrayEquals(byteArrayOf(11), input.read(0uL, 1u))
            assertEquals(1L, Os.lseek(descriptor.fileDescriptor, 0, OsConstants.SEEK_CUR))
            input.close(); input.close()
            assertFalse(descriptor.fileDescriptor.valid())
            assertThrows(AndroidException.InvalidOptions::class.java) { input.read(0uL, 1u) }
            val writable = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
            val output = SelectedDescriptorOutput(writable)
            val bytes = byteArrayOf(42, 43)
            assertEquals(2u, output.write(1uL, bytes))
            assertArrayEquals(byteArrayOf(0, 0), bytes)
            output.finish(); output.close(); output.close()
            assertFalse(writable.fileDescriptor.valid())
            assertArrayEquals(byteArrayOf(11, 42, 43, 14), file.readBytes())
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
        } finally { assertTrue(file.delete()) }
    }
    @Test fun anActualFsyncFailureIsReturnedBeforeDestinationOwnershipEnds() {
        val descriptor = ParcelFileDescriptor.open(File("/dev/null"), ParcelFileDescriptor.MODE_READ_WRITE)
        val output = SelectedDescriptorOutput(descriptor)
        try { assertThrows(AndroidException.StorageIo::class.java) { output.finish() } }
        finally { output.close() }
        assertFalse(descriptor.fileDescriptor.valid())
    }
    @Test fun closingAnUnusedSafOutputDoesNotOpenOrTruncateTheDestination() {
        val directory = File(context.cacheDir, "PUBLIC-selected-fixtures")
        check(directory.isDirectory || directory.mkdirs())
        val file = File(directory, "PUBLIC-${UUID.randomUUID()}.bin")
        val public = "PUBLIC untouched selected destination".toByteArray()
        file.writeBytes(public)
        try {
            val output = SelectedDescriptors.openOutput(context, Uri.parse("content://dev.taypeer.publicselected/${file.name}"))
            output.close(); output.close()
            assertArrayEquals(public, file.readBytes())
            assertThrows(AndroidException.InvalidOptions::class.java) { output.finish() }
            assertArrayEquals(public, file.readBytes())
        } finally { assertTrue(file.delete()) }
    }
}
