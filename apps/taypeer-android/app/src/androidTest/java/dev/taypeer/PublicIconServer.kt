package dev.taypeer

import org.junit.Assert.assertFalse
import java.net.ServerSocket
import java.net.SocketException
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/** Count actual explicit icon HTTP requests using a PUBLIC loopback response only. */
internal class PublicIconServer(private val bytes: ByteArray) : AutoCloseable {
    private val listener = ServerSocket(0, 4, java.net.InetAddress.getByName("127.0.0.1"))
    val requests = AtomicInteger()
    private val failure = AtomicReference<Throwable?>()
    val url = "http://127.0.0.1:${listener.localPort}/PUBLIC.svg"
    private val thread = Thread({
        try {
            while (!listener.isClosed) {
                listener.accept().use { socket ->
                    socket.soTimeout = 10_000
                    val reader = socket.getInputStream().bufferedReader()
                    while (true) { val line = reader.readLine() ?: break; if (line.isEmpty()) break }
                    requests.incrementAndGet()
                    val output = socket.getOutputStream()
                    output.write("HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray())
                    output.write(bytes); output.flush()
                }
            }
        } catch (error: Throwable) {
            if (!(error is SocketException && listener.isClosed)) failure.set(error)
        }
    }, "PUBLIC image loopback server").apply { start() }
    override fun close() {
        listener.close(); thread.join(2_000)
        assertFalse("PUBLIC loopback server failed to stop", thread.isAlive)
        failure.get()?.let { throw AssertionError("PUBLIC loopback response failed", it) }
    }
}
