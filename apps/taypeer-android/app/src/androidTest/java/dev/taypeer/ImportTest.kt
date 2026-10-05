package dev.taypeer

import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.Host
import dev.taypeer.platform.KeystoreCredentials
import dev.taypeer.platform.WorkingCopies
import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.util.UUID

class ImportTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private inline fun fixture(block: (WorkingCopies) -> Unit) {
        val context = instrumentation.targetContext
        val profile = File(context.noBackupFilesDir, "PUBLIC-import-${UUID.randomUUID()}")
        try { Host(profile.path, KeystoreCredentials(context)).use { host -> block(WorkingCopies(context, host)) } }
        finally { assertTrue(profile.deleteRecursively()) }
    }
    @Test fun importsVerifiedCiphertextAndKeepsTheSourceUnchanged() = fixture { copies ->
        val source = instrumentation.context.assets.open("empty.taypeer").use { it.readBytes() }
        val expected = source.copyOf()
        val imported = copies.importStream { ByteArrayInputStream(source) }
        assertArrayEquals(expected, source)
        assertArrayEquals(expected, imported.readBytes())
        // A staged encrypted archive has no authenticated catalog entry or new admission.
        assertFalse(copies.catalog().contains(imported))
    }
    @Test fun interruptedProviderAndInvalidFormatNeverEnterTheCatalog() = fixture { copies ->
        val before = copies.catalog()
        assertThrows(IOException::class.java) {
            copies.importStream { object : InputStream() {
                var read = false
                override fun read(): Int {
                    if (read) throw IOException("PUBLIC-provider-failure")
                    read = true
                    return 1
                }
            } }
        }
        assertEquals(before, copies.catalog())
        assertThrows(dev.taypeer.bridge.AndroidException.InvalidFile::class.java) {
            copies.importStream { ByteArrayInputStream("PUBLIC-invalid-file".toByteArray()) }
        }
        assertEquals(before, copies.catalog())
    }
}
