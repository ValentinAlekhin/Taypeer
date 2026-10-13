package dev.taypeer

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.Host
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import kotlinx.coroutines.*
import java.io.File
import java.util.UUID

/** Native policies cross UniFFI and survive an actual Android Host/Keystore restart. */
@RunWith(AndroidJUnit4::class)
class DeviceSettingsTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test fun policyUpdatesSurviveHostRestartAndInvalidValuesDoNotApply() {
        val profile = File(context.noBackupFilesDir, "PUBLIC-device-policy-${UUID.randomUUID()}")
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                val defaults = host.deviceSettings()
                assertEquals(300u, defaults.idleSeconds)
                assertEquals(30u, defaults.clipboardSeconds)
                assertEquals(17u, host.setIdleSeconds(17u).idleSeconds)
                assertNull(host.setClipboardSeconds(null).clipboardSeconds)
                val applied = host.deviceSettings()
                assertThrows(AndroidException.InvalidOptions::class.java) { host.setIdleSeconds(0u) }
                assertThrows(AndroidException.InvalidOptions::class.java) { host.setClipboardSeconds(0u) }
                assertEquals(applied, host.deviceSettings())
            }
            Host(profile.path, KeystoreCredentials(context)).use { restored ->
                assertEquals(17u, restored.deviceSettings().idleSeconds)
                assertNull(restored.deviceSettings().clipboardSeconds)
                assertEquals(43u, restored.setClipboardSeconds(43u).clipboardSeconds)
            }
            Host(profile.path, KeystoreCredentials(context)).use { restored ->
                assertEquals(43u, restored.deviceSettings().clipboardSeconds)
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    @Test fun corruptPolicyFailsHostCreationInsteadOfUsingAnUnreportedDefault() {
        val profile = File(context.noBackupFilesDir, "PUBLIC-invalid-device-policy-${UUID.randomUUID()}")
        try {
            assertTrue(profile.mkdirs())
            val clipboard = File(profile, "clipboard-policy.json")
            val malformed = "{\"version\":1,\"clipboard\":{\"mode\":\"timed\",\"seconds\":0}}"
            clipboard.writeText(malformed)
            assertThrows(AndroidException.ProfileInvalid::class.java) { Host(profile.path, KeystoreCredentials(context)) }
            assertEquals(malformed, clipboard.readText())
            assertTrue(clipboard.delete())
            File(profile, "session-policy.json").writeText("{\"version\":1,\"policy\":{\"idle_seconds\":0}}")
            assertThrows(AndroidException.ProfileInvalid::class.java) { Host(profile.path, KeystoreCredentials(context)) }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    @Test fun exactRawDeviceNameAndUtf8BoundarySurviveRestartWhileNativeRejectionPreservesBytes() {
        val profile = File(context.noBackupFilesDir, "PUBLIC-device-name-${UUID.randomUUID()}")
        val raw = "  PUBLIC Android телефон α  "
        val boundary = "PUBLIC " + "Ж".repeat(124) + "a"
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                assertEquals("Taypeer", host.deviceSettings().deviceName)
                assertEquals(raw, host.setDeviceName(raw).deviceName)
                assertEquals(256, boundary.toByteArray(Charsets.UTF_8).size)
                assertEquals(boundary, host.setDeviceName(boundary).deviceName)
                val before = File(profile, "device-name.json").readBytes()
                assertThrows(AndroidException.DeviceNameRequired::class.java) { host.setDeviceName(" \t\n ") }
                assertThrows(AndroidException.DeviceNameTooLong::class.java) { host.setDeviceName(boundary + "Ж") }
                assertArrayEquals(before, File(profile, "device-name.json").readBytes())
                assertEquals(boundary, host.deviceSettings().deviceName)
                assertEquals(raw, host.setDeviceName(raw).deviceName)
            }
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                assertEquals(raw, host.deviceSettings().deviceName)
                assertEquals(300u, host.deviceSettings().idleSeconds)
                assertEquals(30u, host.deviceSettings().clipboardSeconds)
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    @Test fun queuedBlurDuringLoadAndLateNameReplyKeepNewerDraftAndWriteEachRevisionOnce() = runBlocking {
        val profile = File(context.noBackupFilesDir, "PUBLIC-device-name-queue-${UUID.randomUUID()}")
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val first = "  PUBLIC first captured name  "
        val second = "PUBLIC newer Android name"
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                val loaded = CompletableDeferred<Unit>()
                val releaseLoad = CompletableDeferred<Unit>()
                val firstWritten = CompletableDeferred<Unit>()
                val releaseFirstReply = CompletableDeferred<Unit>()
                val secondQueued = CompletableDeferred<Unit>()
                val releaseSecondWrite = CompletableDeferred<Unit>()
                val names = java.util.Collections.synchronizedList(mutableListOf<String>())
                val backend = object : PublicSettingsBackend(host) {
                    override suspend fun load(): dev.taypeer.bridge.DeviceSettingsView {
                        val view = super.load(); loaded.complete(Unit); releaseLoad.await(); return view
                    }
                    override suspend fun name(value: String): dev.taypeer.bridge.DeviceSettingsView {
                        names += value
                        if (names.size == 1) {
                            val view = super.name(value); firstWritten.complete(Unit); releaseFirstReply.await(); return view
                        }
                        secondQueued.complete(Unit); releaseSecondWrite.await(); return super.name(value)
                    }
                }
                val state = withContext(Dispatchers.Main) { DeviceSettingsState(backend, scope).also { it.load() } }
                withTimeout(20_000) { loaded.await() }
                withContext(Dispatchers.Main) {
                    state.editName(first); state.finishNameInput(); state.finishNameInput(); state.submitName()
                    assertNull(state.settings); assertEquals(first, state.nameDraft)
                }
                assertEquals(0, names.size)
                releaseLoad.complete(Unit)
                withTimeout(20_000) { firstWritten.await() }
                withContext(Dispatchers.Main) {
                    assertEquals(first, state.nameDraft)
                    assertEquals("Taypeer", state.settings!!.deviceName)
                    state.editName(second); state.submitName(); state.finishNameInput(); state.submitName()
                }
                releaseFirstReply.complete(Unit)
                withTimeout(20_000) { secondQueued.await() }
                withContext(Dispatchers.Main) {
                    assertEquals(first, state.settings!!.deviceName)
                    assertEquals(second, state.nameDraft)
                    assertNull(state.nameFailure)
                }
                assertEquals(first, host.deviceSettings().deviceName)
                releaseSecondWrite.complete(Unit)
                awaitIdle(state)
                withContext(Dispatchers.Main) {
                    state.finishNameInput(); state.submitName()
                    assertEquals(second, state.nameDraft); assertEquals(second, state.settings!!.deviceName)
                }
                awaitIdle(state)
                assertEquals(listOf(first, second), names.toList())
                assertEquals(second, host.deviceSettings().deviceName)
            }
        } finally { scope.cancel(); assertTrue(profile.deleteRecursively()) }
    }

    @Test fun nameBlurQueuedBehindIdleAndClipboardUsesCapturedRawValue() = runBlocking {
        val profile = File(context.noBackupFilesDir, "PUBLIC-device-name-policy-queue-${UUID.randomUUID()}")
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val raw = "  PUBLIC queued policy name β  "
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                val idleWritten = CompletableDeferred<Unit>()
                val releaseIdle = CompletableDeferred<Unit>()
                val clipboardWritten = CompletableDeferred<Unit>()
                val releaseClipboard = CompletableDeferred<Unit>()
                val names = java.util.Collections.synchronizedList(mutableListOf<String>())
                val backend = object : PublicSettingsBackend(host) {
                    override suspend fun idle(seconds: UInt): dev.taypeer.bridge.DeviceSettingsView {
                        val view = super.idle(seconds); idleWritten.complete(Unit); releaseIdle.await(); return view
                    }
                    override suspend fun clipboard(seconds: UInt?): dev.taypeer.bridge.DeviceSettingsView {
                        val view = super.clipboard(seconds); clipboardWritten.complete(Unit); releaseClipboard.await(); return view
                    }
                    override suspend fun name(value: String): dev.taypeer.bridge.DeviceSettingsView { names += value; return super.name(value) }
                }
                val state = withContext(Dispatchers.Main) { DeviceSettingsState(backend, scope).also { it.load() } }
                awaitIdle(state)
                withContext(Dispatchers.Main) {
                    state.applyIdle(41u); state.applyClipboard(null)
                    state.editName(raw); state.finishNameInput(); state.finishNameInput()
                }
                withTimeout(20_000) { idleWritten.await() }
                assertEquals(0, names.size)
                releaseIdle.complete(Unit)
                withTimeout(20_000) { clipboardWritten.await() }
                withContext(Dispatchers.Main) { assertEquals(raw, state.nameDraft); assertEquals(41u, state.settings!!.idleSeconds) }
                assertEquals(0, names.size)
                releaseClipboard.complete(Unit)
                awaitIdle(state)
                assertEquals(listOf(raw), names.toList())
                withContext(Dispatchers.Main) {
                    assertEquals(raw, state.settings!!.deviceName); assertEquals(raw, state.nameDraft)
                    assertEquals(41u, state.settings!!.idleSeconds); assertNull(state.settings!!.clipboardSeconds)
                }
                assertEquals(raw, host.deviceSettings().deviceName)
            }
        } finally { scope.cancel(); assertTrue(profile.deleteRecursively()) }
    }

    @Test fun lostNamePublicationRequiresConfirmedSetterEvenAfterCacheReadOrReenteringCachedName() = runBlocking {
        val profile = File(context.noBackupFilesDir, "PUBLIC-device-name-lost-reply-${UUID.randomUUID()}")
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val raw = "PUBLIC durable name with lost reply"
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                var loseReply = true
                val names = java.util.Collections.synchronizedList(mutableListOf<String>())
                val backend = object : PublicSettingsBackend(host) {
                    override suspend fun name(value: String): dev.taypeer.bridge.DeviceSettingsView {
                        names += value
                        val view = super.name(value)
                        if (loseReply) throw AndroidException.CommitUncertain()
                        return view
                    }
                }
                val state = withContext(Dispatchers.Main) { DeviceSettingsState(backend, scope).also { it.load() } }
                awaitIdle(state)
                withContext(Dispatchers.Main) { state.editName(raw); state.submitName(); state.finishNameInput() }
                awaitIdle(state)
                withContext(Dispatchers.Main) {
                    assertEquals("Taypeer", state.settings!!.deviceName)
                    assertEquals(raw, state.nameDraft); assertEquals(DeviceNameFailure.UNCERTAIN, state.nameFailure)
                }
                assertEquals(raw, host.deviceSettings().deviceName)
                withContext(Dispatchers.Main) { state.load() }
                awaitIdle(state)
                withContext(Dispatchers.Main) {
                    assertEquals(raw, state.settings!!.deviceName); assertEquals(raw, state.nameDraft)
                    assertEquals(DeviceNameFailure.UNCERTAIN, state.nameFailure)
                }
                loseReply = false
                withContext(Dispatchers.Main) { state.submitName(); state.finishNameInput() }
                awaitIdle(state)
                assertEquals(listOf(raw, raw), names.toList())
                withContext(Dispatchers.Main) { assertNull(state.nameFailure) }

                val other = "PUBLIC differently published name"
                loseReply = true
                withContext(Dispatchers.Main) { state.editName(other); state.submitName() }
                awaitIdle(state)
                withContext(Dispatchers.Main) { assertEquals(raw, state.settings!!.deviceName) }
                assertEquals(other, host.deviceSettings().deviceName)
                val uncertainBytes = File(profile, "device-name.json").readBytes()
                loseReply = false
                withContext(Dispatchers.Main) { state.editName(raw); state.submitName(); state.finishNameInput() }
                awaitIdle(state)
                assertEquals(listOf(raw, raw, other, raw), names.toList())
                assertEquals(raw, host.deviceSettings().deviceName)
                assertFalse(uncertainBytes.contentEquals(File(profile, "device-name.json").readBytes()))
                assertEquals(raw, org.json.JSONObject(File(profile, "device-name.json").readText()).getString("name"))
                withContext(Dispatchers.Main) { assertEquals(raw, state.nameDraft); assertNull(state.nameFailure) }
            }
        } finally { scope.cancel(); assertTrue(profile.deleteRecursively()) }
    }

    @Test fun lateUncertainReplyStillRequiresWritingReenteredOldCachedName() = runBlocking {
        val profile = File(context.noBackupFilesDir, "PUBLIC-device-name-late-unknown-${UUID.randomUUID()}")
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val other = "PUBLIC published name before lost confirmation"
        try {
            Host(profile.path, KeystoreCredentials(context)).use { host ->
                val published = CompletableDeferred<Unit>()
                val releaseFailure = CompletableDeferred<Unit>()
                val names = java.util.Collections.synchronizedList(mutableListOf<String>())
                val backend = object : PublicSettingsBackend(host) {
                    override suspend fun name(value: String): dev.taypeer.bridge.DeviceSettingsView {
                        names += value
                        val result = super.name(value)
                        if (names.size == 1) {
                            published.complete(Unit); releaseFailure.await()
                            throw AndroidException.CommitUncertain()
                        }
                        return result
                    }
                }
                val state = withContext(Dispatchers.Main) { DeviceSettingsState(backend, scope).also { it.load() } }
                awaitIdle(state)
                withContext(Dispatchers.Main) { state.editName(other); state.submitName() }
                withTimeout(20_000) { published.await() }
                withContext(Dispatchers.Main) { state.editName("Taypeer") }
                val uncertainBytes = File(profile, "device-name.json").readBytes()
                assertEquals(other, host.deviceSettings().deviceName)
                releaseFailure.complete(Unit)
                awaitIdle(state)
                withContext(Dispatchers.Main) {
                    assertEquals("Taypeer", state.settings!!.deviceName); assertEquals("Taypeer", state.nameDraft)
                    assertNull(state.nameFailure)
                    state.submitName(); state.finishNameInput()
                }
                awaitIdle(state)
                assertEquals(listOf(other, "Taypeer"), names.toList())
                assertEquals("Taypeer", host.deviceSettings().deviceName)
                assertFalse(uncertainBytes.contentEquals(File(profile, "device-name.json").readBytes()))
                assertEquals("Taypeer", org.json.JSONObject(File(profile, "device-name.json").readText()).getString("name"))
            }
        } finally { scope.cancel(); assertTrue(profile.deleteRecursively()) }
    }

    private suspend fun awaitIdle(state: DeviceSettingsState) = withTimeout(20_000) { while (state.busy) delay(10) }

    private open class PublicSettingsBackend(private val host: Host) : DeviceSettingsBackend {
        override suspend fun load() = host.deviceSettings()
        override suspend fun idle(seconds: UInt) = host.setIdleSeconds(seconds)
        override suspend fun clipboard(seconds: UInt?) = host.setClipboardSeconds(seconds)
        override suspend fun name(value: String) = host.setDeviceName(value)
    }
}
