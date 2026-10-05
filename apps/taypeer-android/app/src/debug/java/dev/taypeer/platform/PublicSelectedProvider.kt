package dev.taypeer.platform

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import java.io.File

/** Debug-only, private fixture provider exercises SAF without exposing application documents. */
class PublicSelectedProvider : ContentProvider() {
    override fun onCreate() = true
    override fun getType(uri: Uri) = "application/octet-stream"
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor {
        val name = name(uri)
        val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        return MatrixCursor(columns).apply {
            val values: Array<Any?> = columns.map { column -> when (column) {
                OpenableColumns.DISPLAY_NAME -> name
                OpenableColumns.SIZE -> if (name == "PUBLIC-unknown") null else file(name).length()
                else -> null
            } }.toTypedArray()
            addRow(values)
        }
    }
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        val name = name(uri)
        if (name == "PUBLIC-unknown") {
            val pipe = ParcelFileDescriptor.createPipe()
            pipe[1].close()
            return pipe[0]
        }
        require(mode == "r" || mode == "wt")
        return ParcelFileDescriptor.open(file(name), ParcelFileDescriptor.parseMode(mode))
    }
    private fun name(uri: Uri): String {
        val name = uri.pathSegments.singleOrNull() ?: error("Invalid public fixture")
        require(name.startsWith("PUBLIC-") && name.length <= 128 && name.all { it.isLetterOrDigit() || it == '-' || it == '.' })
        return name
    }
    private fun file(name: String): File {
        val directory = File(requireNotNull(context).cacheDir, "PUBLIC-selected-fixtures")
        check(directory.isDirectory || directory.mkdirs())
        return File(directory, name)
    }
    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?) = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?) = throw UnsupportedOperationException()
}
