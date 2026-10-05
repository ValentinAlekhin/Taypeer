package dev.taypeer

import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.RemoteException
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.CipherCommit
import dev.taypeer.bridge.CipherSeed
import dev.taypeer.platform.BinderDocumentPersistence
import dev.taypeer.platform.CipherAuthorResult
import dev.taypeer.platform.CipherFileResult
import dev.taypeer.platform.CiphertextDescriptors
import dev.taypeer.platform.ICipherPersistence
import dev.taypeer.platform.anonymousCiphertext
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Durability failures survive the private platform boundary, including a lost mutation reply. */
@RunWith(AndroidJUnit4::class)
class CipherBinderTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private open class Port : ICipherPersistence.Stub() {
        override fun createTemporary() = CipherFileResult(1, null)
        override fun snapshot() = Bundle().apply { putInt("status", 1) }
        override fun create(controls: ByteArray, objects: Array<ParcelFileDescriptor>, checkpoint: String, baseline: String) = 1
        override fun commit(expected: String, control: String, controls: ByteArray, objects: Array<ParcelFileDescriptor>, remove: Array<String>, checkpoint: String, baseline: String, journal: ByteArray) = Bundle().apply { putInt("status", 1) }
        override fun author(authenticated: String?) = CipherAuthorResult(1, null)
        override fun transportPublic() = "PUBLIC transport"
        override fun saveDraft(file: ParcelFileDescriptor) = 1
        override fun loadDraft() = CipherFileResult(1, null)
        override fun discardDraft() = 1
    }
    private val seed = CipherSeed(byteArrayOf(), listOf(), "PUBLIC checkpoint", "PUBLIC baseline")
    private val commit = CipherCommit("PUBLIC expected", "PUBLIC control", byteArrayOf(), listOf(), listOf(), "PUBLIC checkpoint", "PUBLIC baseline", byteArrayOf())
    @Test fun explicitStorageCategoriesArePreservedAndOnlyLostMutationRepliesAreUncertain() {
        CiphertextDescriptors { anonymousCiphertext(context) }.use { files ->
            val changed = BinderDocumentPersistence(object : Port() {
                override fun create(controls: ByteArray, objects: Array<ParcelFileDescriptor>, checkpoint: String, baseline: String) = 4
                override fun commit(expected: String, control: String, controls: ByteArray, objects: Array<ParcelFileDescriptor>, remove: Array<String>, checkpoint: String, baseline: String, journal: ByteArray) = Bundle().apply { putInt("status", 2) }
            }, files)
            assertThrows(AndroidException.AlreadyExists::class.java) { changed.create(seed) }
            assertThrows(AndroidException.StorageChanged::class.java) { changed.commit(commit) }
            assertThrows(AndroidException.StorageIo::class.java) { changed.snapshot() }
            val lost = BinderDocumentPersistence(object : Port() {
                override fun create(controls: ByteArray, objects: Array<ParcelFileDescriptor>, checkpoint: String, baseline: String): Int = throw RemoteException()
                override fun snapshot(): Bundle = throw RemoteException()
            }, files)
            assertThrows(AndroidException.CommitUncertain::class.java) { lost.create(seed) }
            assertThrows(AndroidException.Runtime::class.java) { lost.snapshot() }
        }
    }
    @Test fun rejectedSnapshotClosesItsReceivedDescriptor() {
        val received = anonymousCiphertext(context)
        CiphertextDescriptors { anonymousCiphertext(context) }.use { files ->
            val port = BinderDocumentPersistence(object : Port() {
                override fun snapshot() = Bundle().apply {
                    putInt("status", 0)
                    putParcelable("file", received)
                    putString("fingerprint", "PUBLIC fingerprint")
                    // Root and working-copy binding are missing: ownership must still end.
                }
            }, files)
            assertThrows(AndroidException.InvalidFile::class.java) { port.snapshot() }
            assertFalse(received.fileDescriptor.valid())
        }
    }
    @Test fun malformedSnapshotAfterSuccessfulCommitIsUncertainAndClosesTheReceivedFile() {
        val received = anonymousCiphertext(context)
        CiphertextDescriptors { anonymousCiphertext(context) }.use { files ->
            val port = BinderDocumentPersistence(object : Port() {
                override fun commit(expected: String, control: String, controls: ByteArray, objects: Array<ParcelFileDescriptor>, remove: Array<String>, checkpoint: String, baseline: String, journal: ByteArray) = Bundle().apply {
                    putInt("status", 0)
                    putParcelable("file", received)
                    putString("fingerprint", "PUBLIC committed fingerprint")
                }
            }, files)
            assertThrows(AndroidException.CommitUncertain::class.java) { port.commit(commit) }
            assertFalse(received.fileDescriptor.valid())
            val missingStatusFile = anonymousCiphertext(context)
            val missingStatus = BinderDocumentPersistence(object : Port() {
                override fun commit(expected: String, control: String, controls: ByteArray, objects: Array<ParcelFileDescriptor>, remove: Array<String>, checkpoint: String, baseline: String, journal: ByteArray) = Bundle().apply {
                    putParcelable("file", missingStatusFile)
                }
            }, files)
            assertThrows(AndroidException.CommitUncertain::class.java) { missingStatus.commit(commit) }
            assertFalse(missingStatusFile.fileDescriptor.valid())
            assertThrows(AndroidException.StorageIo::class.java) { BinderDocumentPersistence(Port(), files).commit(commit) }
        }
    }
}
