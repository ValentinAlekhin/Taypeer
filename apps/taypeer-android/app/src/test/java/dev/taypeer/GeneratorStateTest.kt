package dev.taypeer

import dev.taypeer.bridge.GeneratedQuality
import dev.taypeer.bridge.GeneratedSecretView
import dev.taypeer.bridge.PasswordOptions
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.test.*
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class GeneratorStateTest {
    private val dispatcher = StandardTestDispatcher()
    private data class Request(val length: UShort?, val words: UByte?, val result: CompletableDeferred<GeneratedSecretView>)
    private class Backend : GeneratorBackend {
        val requests = mutableListOf<Request>()
        private suspend fun request(length: UShort?, words: UByte?): GeneratedSecretView {
            val pending = Request(length, words, CompletableDeferred())
            requests += pending
            // Simulate a native operation that cannot stop once dispatched.
            return withContext(NonCancellable) { pending.result.await() }
        }
        override suspend fun password(options: PasswordOptions) = request(options.length, null)
        override suspend fun passphrase(words: UByte, separator: String) = request(null, words)
    }
    @Before fun mainDispatcher() { Dispatchers.setMain(dispatcher) }
    @After fun resetDispatcher() { Dispatchers.resetMain() }
    private fun result(name: String, bits: Double = 77.55, quality: GeneratedQuality = GeneratedQuality.HIGH) =
        GeneratedSecretView("PUBLIC $name", bits, quality)

    @Test fun invalidInputImmediatelyRevokesThePreviousResultAndMetadata() = runTest(dispatcher) {
        val backend = Backend(); val state = GeneratorState(backend, dispatcher)
        state.generate(); runCurrent()
        backend.requests.single().result.complete(result("first")); runCurrent()
        assertEquals("PUBLIC first", state.generated)
        state.revealed = true
        state.changeLength("")
        assertEquals("", state.generated); assertEquals(0.0, state.entropyBits, 0.0)
        assertFalse(state.revealed); assertFalse(state.running); assertTrue(state.error)
        advanceUntilIdle()
        assertEquals(1, backend.requests.size)
        state.lock()
    }
    @Test fun changedOptionsDebounceAndOnlyTheLatestRequestMayPublish() = runTest(dispatcher) {
        val backend = Backend(); val state = GeneratorState(backend, dispatcher)
        state.generate(); runCurrent()
        state.changeLength("40"); state.changeLength("41")
        assertEquals("", state.generated)
        advanceTimeBy(151); runCurrent()
        assertEquals(2, backend.requests.size)
        assertEquals(41.toUShort(), backend.requests.last().length)
        backend.requests.last().result.complete(result("latest", 123.45, GeneratedQuality.MEDIUM)); runCurrent()
        backend.requests.first().result.complete(result("stale")); runCurrent()
        assertEquals("PUBLIC latest", state.generated)
        assertEquals(123.45, state.entropyBits, 0.0)
        assertEquals(GeneratedQuality.MEDIUM, state.quality)
        assertFalse(state.running)
        state.lock()
    }
    @Test fun nativeFailureNeverLeavesAnOldResultAvailableForCopying() = runTest(dispatcher) {
        val backend = Backend(); val state = GeneratorState(backend, dispatcher)
        state.generate(); runCurrent()
        backend.requests.single().result.complete(result("old")); runCurrent()
        state.changeLength("0")
        assertEquals("", state.generated)
        advanceTimeBy(151); runCurrent()
        backend.requests.last().result.completeExceptionally(IllegalArgumentException("PUBLIC invalid options")); runCurrent()
        assertTrue(state.error); assertFalse(state.running); assertEquals("", state.generated)
        state.lock()
    }
    @Test fun lockingRevokesAnInFlightPassphraseAndRejectsItsLateCompletion() = runTest(dispatcher) {
        val backend = Backend(); val state = GeneratorState(backend, dispatcher)
        state.selectMode(GeneratorMode.PASSPHRASE)
        advanceTimeBy(151); runCurrent()
        assertEquals(6.toUByte(), backend.requests.single().words)
        state.lock()
        backend.requests.single().result.complete(result("late phrase")); runCurrent()
        assertEquals("", state.generated); assertEquals(0.0, state.entropyBits, 0.0)
        assertFalse(state.running); assertFalse(state.error)
    }
}
