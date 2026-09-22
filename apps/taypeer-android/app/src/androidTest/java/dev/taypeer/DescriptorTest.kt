package dev.taypeer

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.IBinder
import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.DescriptorProbeService
import dev.taypeer.platform.IDescriptorProbe
import dev.taypeer.platform.ITemporaryFiles
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

@RunWith(AndroidJUnit4::class)
class DescriptorTest {
    @Test fun isolatedRustVerifiesTransferredArchiveUsingAnonymousCiphertextFiles() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val directory = File(context.cacheDir, "PUBLIC-descriptors-${UUID.randomUUID()}")
        assertTrue(directory.mkdir())
        val source = File(directory, "PUBLIC.taypeer")
        instrumentation.context.assets.open("populated.taypeer").use { input ->
            source.outputStream().use { input.copyTo(it) }
        }
        val before = source.readBytes()
        val executor = Executors.newSingleThreadExecutor()
        val connected = CountDownLatch(1)
        lateinit var remote: IDescriptorProbe
        val connection = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName, binder: IBinder) {
                remote = IDescriptorProbe.Stub.asInterface(binder)
                connected.countDown()
            }
            override fun onServiceDisconnected(name: ComponentName) {}
        }
        val allocated = AtomicInteger()
        val files = object : ITemporaryFiles.Stub() {
            override fun create(): ParcelFileDescriptor {
                val file = File.createTempFile("PUBLIC-ciphertext-", ".tmp", directory)
                val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_WRITE)
                check(file.delete()) // Binder transfers an anonymous inode, never a reusable pathname.
                allocated.incrementAndGet()
                return descriptor
            }
        }
        var bound = false
        try {
            bound = context.bindIsolatedService(Intent(context, DescriptorProbeService::class.java),
                Context.BIND_AUTO_CREATE, "descriptor_${UUID.randomUUID().toString().replace("-", "")}",
                executor, connection)
            assertTrue(bound)
            assertTrue(connected.await(10, TimeUnit.SECONDS))
            val count = ParcelFileDescriptor.open(source, ParcelFileDescriptor.MODE_READ_ONLY).use {
                remote.verify(it, files)
            }
            assertTrue(count > 0)
            assertEquals(count, allocated.get().toLong())
            assertArrayEquals(before, source.readBytes())
            assertEquals(listOf(source.name), directory.listFiles()!!.map { it.name })
            val denied = object : ITemporaryFiles.Stub() {
                override fun create(): ParcelFileDescriptor = throw IllegalStateException("PUBLIC denied allocation")
            }
            ParcelFileDescriptor.open(source, ParcelFileDescriptor.MODE_READ_ONLY).use {
                assertThrows(IllegalStateException::class.java) { remote.verify(it, denied) }
            }
            assertArrayEquals(before, source.readBytes())
            source.writeBytes(before.copyOf().also { it[0] = (it[0].toInt() xor 1).toByte() })
            ParcelFileDescriptor.open(source, ParcelFileDescriptor.MODE_READ_ONLY).use {
                assertThrows(IllegalStateException::class.java) { remote.verify(it, files) }
            }
        } finally {
            if (bound) context.unbindService(connection)
            executor.shutdownNow()
            assertTrue(directory.deleteRecursively())
        }
    }
}
