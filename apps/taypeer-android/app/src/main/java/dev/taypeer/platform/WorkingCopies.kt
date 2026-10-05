package dev.taypeer.platform

import android.content.Context
import android.net.Uri
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.inspectImport
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream

/** SAF is read-only input. The common Rust host owns internal destinations and catalog rules. */
class WorkingCopies(context: Context, private val host: Host) {
    private val resolver = context.contentResolver
    private val staging = File(context.noBackupFilesDir, "import-staging")

    /** Off the UI thread; authentication publishes the common catalog later. */
    fun import(uri: Uri): File = importStream {
        resolver.openInputStream(uri) ?: throw IOException("Provider unavailable")
    }
    internal fun importStream(open: () -> InputStream): File {
        if (!staging.isDirectory && !staging.mkdirs()) throw IOException("Staging unavailable")
        val temporary = File.createTempFile("cipher-", ".pending", staging)
        try {
            open().use { source ->
                FileOutputStream(temporary).use { destination ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        if (Thread.currentThread().isInterrupted) throw IOException("Import cancelled")
                        val count = source.read(buffer)
                        if (count < 0) break
                        destination.write(buffer, 0, count)
                    }
                    destination.fd.sync()
                }
            }
            inspectImport(temporary.path)
            if (Thread.currentThread().isInterrupted) throw IOException("Import cancelled")
            return File(host.stageImport(temporary.path).path)
        } finally { temporary.delete() }
    }
    fun catalog(): List<File> = host.workingCopies().map { File(it.path) }
}
