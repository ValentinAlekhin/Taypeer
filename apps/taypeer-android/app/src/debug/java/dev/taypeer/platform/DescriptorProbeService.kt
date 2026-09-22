package dev.taypeer.platform

import android.app.Service
import android.content.Intent
import android.os.IBinder
import android.os.ParcelFileDescriptor
import android.os.Process
import dev.taypeer.bridge.CiphertextArchive

/** Debug acceptance harness; production document lifecycle remains in DocumentService. */
class DescriptorProbeService : Service() {
    override fun onBind(intent: Intent): IBinder = object : IDescriptorProbe.Stub() {
        override fun verify(archive: ParcelFileDescriptor, files: ITemporaryFiles): Long {
            try {
                CiphertextDescriptors { files.create() }.use { port ->
                    CiphertextArchive(port.accept(archive), port).use { verified ->
                        check(verified.databaseId().isNotEmpty())
                        return verified.verifyObjects().toLong()
                    }
                }
            } catch (_: Exception) {
                // AIDL cannot marshal arbitrary UniFFI exceptions; preserve a definite failure.
                throw IllegalStateException("Ciphertext verification failed")
            }
        }
    }
    override fun onUnbind(intent: Intent): Boolean {
        Process.killProcess(Process.myPid())
        return false
    }
}
