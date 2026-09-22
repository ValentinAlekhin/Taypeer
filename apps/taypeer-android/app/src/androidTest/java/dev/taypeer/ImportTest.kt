package dev.taypeer

import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.WorkingCopies
import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream

class ImportTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val copies = WorkingCopies(instrumentation.targetContext)

    @Test fun importsVerifiedCiphertextAndKeepsTheSourceUnchanged() {
        val source = instrumentation.context.assets.open("empty.taypeer").use { it.readBytes() }
        val expected = source.copyOf()
        val imported = copies.importStream { ByteArrayInputStream(source) }
        try {
            assertArrayEquals(expected, source)
            assertArrayEquals(expected, imported.readBytes())
            assertTrue(copies.catalog().contains(imported))
        } finally { assertTrue(imported.delete()) }
    }

    @Test fun interruptedProviderAndInvalidFormatNeverEnterTheCatalog() {
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
