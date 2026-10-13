package dev.taypeer

import org.junit.Assert.*
import java.net.ServerSocket
import java.net.SocketException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/** Real PUBLIC page discovery followed by a declared original; optionally hold the image response. */
internal class PublicFaviconServer(private val bytes: ByteArray, holdImage: Boolean = false) : AutoCloseable {
    private val listener = ServerSocket(0, 4, java.net.InetAddress.getByName("127.0.0.1"))
    val requests = AtomicInteger()
    val entered = CountDownLatch(1)
    val release = CountDownLatch(if (holdImage) 1 else 0)
    val url = "http://127.0.0.1:${listener.localPort}/PUBLIC-page"
    private val paths = mutableListOf<String>()
    private val failure = AtomicReference<Throwable?>()
    fun requestPaths(): List<String> = synchronized(paths) { paths.toList() }
    private val thread = Thread({
        try {
            while (!listener.isClosed) listener.accept().use { socket ->
                socket.soTimeout = 10_000
                val input = socket.getInputStream().bufferedReader()
                val first = input.readLine() ?: return@use
                var complete = false
                var headerSize = first.length
                while (headerSize <= 8192) {
                    val line = input.readLine() ?: break
                    headerSize += line.length
                    check(!line.startsWith("Authorization:", ignoreCase = true) &&
                        !line.startsWith("Cookie:", ignoreCase = true) &&
                        !line.startsWith("Referer:", ignoreCase = true)) { "PUBLIC favicon request must not send site credentials" }
                    if (line.isEmpty()) { complete = true; break }
                }
                check(complete) { "PUBLIC request must have complete bounded headers" }
                check(first.startsWith("GET ")) { "PUBLIC favicon acquisition must use GET" }
                val path = first.split(' ').getOrNull(1) ?: error("PUBLIC request route is missing")
                check(path == "/PUBLIC-page" || path == "/PUBLIC.svg") { "PUBLIC fixture received an unexpected route" }
                synchronized(paths) { paths.add(path) }
                requests.incrementAndGet()
                val body: ByteArray
                val type: String
                if (path == "/PUBLIC-page") {
                    body = """<html><head><link rel="icon" href="/PUBLIC.svg"></head></html>""".toByteArray()
                    type = "text/html"
                } else {
                    entered.countDown()
                    check(release.await(15, TimeUnit.SECONDS)) { "PUBLIC held image must be released" }
                    body = bytes
                    type = "image/svg+xml"
                }
                val output = socket.getOutputStream()
                output.write("HTTP/1.1 200 OK\r\nContent-Type: $type\r\nContent-Length: ${body.size}\r\nConnection: close\r\n\r\n".toByteArray())
                output.write(body); output.flush()
            }
        } catch (error: Throwable) {
            if (!(error is SocketException && listener.isClosed)) failure.set(error)
        }
    }, "PUBLIC favicon discovery server").apply { start() }
    override fun close() {
        release.countDown(); listener.close(); thread.join(2_000)
        assertFalse("PUBLIC favicon server must release resources", thread.isAlive)
        failure.get()?.let { throw AssertionError("PUBLIC favicon response failed", it) }
    }
}
