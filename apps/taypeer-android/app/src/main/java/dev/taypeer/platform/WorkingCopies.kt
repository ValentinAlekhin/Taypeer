package dev.taypeer.platform

import android.content.Context
import android.net.Uri
import android.system.Os
import android.system.OsConstants
import dev.taypeer.bridge.inspectImport
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream
import java.util.UUID

/** Ciphertext-only import. The selected provider file is opened read-only and never replaced. */
class WorkingCopies(context: Context) {
    private val resolver = context.contentResolver
    private val directory = File(context.noBackupFilesDir, "databases")

    /** Call off the UI thread. Only completed .taypeer files enter the catalog. */
    fun import(uri: Uri): File = importStream {
        resolver.openInputStream(uri) ?: throw IOException("Provider unavailable")
    }

    internal fun importStream(open: () -> InputStream): File {
        check(directory.isDirectory || directory.mkdirs())
        // Reserve a fresh directory atomically. Android SELinux forbids app hard links.
        val copy = File(directory, UUID.randomUUID().toString())
        check(copy.mkdir())
        val temporary = File(copy, "import.pending")
        val published = File(copy, "working.taypeer")
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
            // Only this import owns the freshly reserved directory and final filename.
            check(!published.exists())
            Os.rename(temporary.path, published.path)
            syncDirectory(copy)
            syncDirectory(directory)
            return published
        } finally {
            if (temporary.exists()) temporary.delete()
            if (!published.exists()) copy.delete()
        }
    }

    private fun syncDirectory(path: File) {
        val descriptor = Os.open(path.path, OsConstants.O_RDONLY, 0)
        try { Os.fsync(descriptor) } finally { Os.close(descriptor) }
    }

    fun catalog(): List<File> {
        if (!directory.exists()) return emptyList()
        return directory.listFiles()?.filter { it.isDirectory }
            ?.map { File(it, "working.taypeer") }?.filter { it.isFile }
            ?.sortedBy { it.parentFile?.name } ?: throw IOException("Catalog unavailable")
    }
}
