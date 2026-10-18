package dev.taypeer

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.Host
import dev.taypeer.bridge.RelayPreference
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.UUID

/** Actual UniFFI preferences acknowledge persistence separately from the pending UI choice. */
class RelaySettingsStateTest {
    @get:Rule val compose = createComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun withState(host: Host, block: (RelaySettingsState) -> Unit) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        try {
            val state = RelaySettingsState(CompletableDeferred(host), scope)
            compose.runOnIdle { state.load() }
            compose.waitUntil(10_000) { !state.busy }
            assertFalse(state.failed)
            block(state)
        } finally { scope.cancel() }
    }

    @Test fun appliedChoiceSurvivesActualHostRestartAndInvalidAddressLeavesTheConfirmedChoice() {
        val profile = File(context.noBackupFilesDir, "PUBLIC-relay-state-${UUID.randomUUID()}")
        val exact = RelayPreference.Custom("https://relay.example.test/PUBLIC/path?Keep=Case")
        try {
            Host.portable(profile.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                withState(host) { state ->
                    assertEquals(RelayPreference.Disabled, state.preference)
                    compose.runOnIdle {
                        state.apply(exact)
                        assertTrue(state.busy)
                        assertEquals(RelayPreference.Disabled, state.preference)
                    }
                    compose.waitUntil(10_000) { !state.busy }
                    assertFalse(state.failed)
                    assertEquals(exact, state.preference)
                    assertEquals(exact, host.relayPreference())
                    val committed = File(profile, "network.json").readBytes()
                    compose.runOnIdle { state.apply(RelayPreference.Custom("PUBLIC invalid address")) }
                    compose.waitUntil(10_000) { !state.busy }
                    assertTrue(state.failed)
                    assertEquals(exact, state.preference)
                    assertEquals(exact, host.relayPreference())
                    assertArrayEquals(committed, File(profile, "network.json").readBytes())
                    compose.runOnIdle { state.load() }
                    compose.waitUntil(10_000) { !state.busy }
                    assertFalse(state.failed)
                    assertEquals(exact, state.preference)
                }
            }
            Host.portable(profile.path).use { restored ->
                withState(restored) { assertEquals(exact, it.preference) }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    @Test fun damagedPreferenceCannotDefaultOrApplyAndExplicitReloadRecoversOnlyConfirmedData() {
        val profile = File(context.noBackupFilesDir, "PUBLIC-relay-failure-${UUID.randomUUID()}")
        try {
            Host.portable(profile.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                withState(host) { state ->
                    val file = File(profile, "network.json")
                    val confirmed = file.readBytes()
                    val malformed = "PUBLIC malformed relay settings".toByteArray()
                    file.writeBytes(malformed)
                    compose.runOnIdle { state.load() }
                    compose.waitUntil(10_000) { !state.busy }
                    assertTrue(state.failed)
                    assertEquals(RelayPreference.Disabled, state.preference)
                    compose.runOnIdle { state.apply(RelayPreference.Public) }
                    compose.waitUntil(10_000) { !state.busy }
                    assertTrue(state.failed)
                    assertEquals(RelayPreference.Disabled, state.preference)
                    assertArrayEquals(malformed, file.readBytes())
                    file.writeBytes(confirmed) // Public fixture repair, never a product fallback.
                    compose.runOnIdle { state.load() }
                    compose.waitUntil(10_000) { !state.busy }
                    assertFalse(state.failed)
                    assertEquals(RelayPreference.Disabled, state.preference)
                }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }
}
