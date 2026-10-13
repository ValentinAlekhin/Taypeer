package dev.taypeer

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import dev.taypeer.bridge.RelayPreference
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class RelaySettingsUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun choosingModeRequiresApplyAndPreservesExactCustomInput() {
        var applied: RelayPreference? = null
        var preference by mutableStateOf<RelayPreference>(RelayPreference.Public)
        compose.setContent { TaypeerTheme { RelaySettingsContent(preference, false, false, { applied = it; preference = it }, {}, {}) } }
        compose.onNodeWithTag("relay-disabled").performClick()
        compose.runOnIdle { assertNull(applied) }
        compose.onNodeWithTag("apply-relay").performClick()
        compose.runOnIdle { assertEquals(RelayPreference.Disabled, applied) }
        compose.onNodeWithTag("relay-custom").performClick()
        val url = "https://relay.example.test/PUBLIC-path?case=KeepThis"
        compose.onNodeWithTag("relay-url").performTextReplacement(url)
        compose.onNodeWithTag("apply-relay").performClick()
        compose.runOnIdle { assertEquals(RelayPreference.Custom(url), applied) }
    }

    @Test fun nativeFailureKeepsEnteredAddressAndBusyStateBlocksDuplicateApply() {
        var busy by mutableStateOf(false)
        var failed by mutableStateOf(false)
        var calls = 0
        val url = "PUBLIC invalid relay address"
        compose.setContent { TaypeerTheme { RelaySettingsContent(RelayPreference.Public, busy, failed, { calls++; busy = true }, {}, {}) } }
        compose.onNodeWithTag("relay-custom").performClick()
        compose.onNodeWithTag("relay-url").performTextReplacement(url)
        compose.onNodeWithTag("apply-relay").performClick()
        compose.onNodeWithTag("apply-relay").assertIsNotEnabled()
        compose.onNodeWithTag("relay-disabled").assertIsNotEnabled()
        compose.runOnIdle { busy = false; failed = true; assertEquals(1, calls) }
        compose.onNodeWithTag("relay-error").assertExists()
        compose.onNodeWithTag("relay-url").assertTextEquals(url)
        compose.onNodeWithTag("apply-relay").performClick()
        compose.runOnIdle { assertEquals(2, calls) }
    }

    @Test fun unknownPreferenceHasNoSelectedModeAndCannotApply() {
        var preference by mutableStateOf<RelayPreference?>(null)
        var calls = 0
        compose.setContent { TaypeerTheme { RelaySettingsContent(preference, false, false, { calls++ }, {}, {}) } }
        listOf("relay-public", "relay-disabled", "relay-custom").forEach {
            compose.onNodeWithTag(it).assertIsNotSelected().assertIsNotEnabled()
        }
        compose.onNodeWithTag("apply-relay").assertIsNotEnabled()
        compose.runOnIdle { preference = RelayPreference.Disabled }
        compose.onNodeWithTag("relay-disabled").assertIsSelected()
        compose.onNodeWithTag("apply-relay").assertIsEnabled()
        compose.runOnIdle { assertEquals(0, calls) }
    }

    @Test fun selectorShowsAcknowledgedValueAndCustomActionDoesNotApply() {
        var preference by mutableStateOf<RelayPreference?>(null)
        var busy by mutableStateOf(false)
        var failed by mutableStateOf(false)
        var requested: RelayPreference? = null
        var customCalls = 0
        compose.setContent { TaypeerTheme(language = UiLanguage.ENGLISH) {
            Column { RelayPreferenceFields(preference, busy, failed,
                { requested = it; busy = true }, { customCalls++ }, {}) }
        } }
        compose.onNodeWithTag("settings-relay").assertIsNotEnabled()
        compose.onNodeWithTag("settings-custom-relay").assertDoesNotExist()
        compose.runOnIdle { preference = RelayPreference.Public }
        compose.onNodeWithTag("settings-relay").assertTextContains("Public relays")
        compose.onNodeWithTag("settings-relay").performClick()
        compose.onNodeWithTag("settings-relay-disabled").performClick()
        compose.runOnIdle { assertEquals(RelayPreference.Disabled, requested); assertEquals(RelayPreference.Public, preference) }
        compose.onNodeWithTag("settings-relay").assertIsNotEnabled().assertTextContains("Public relays")
        compose.runOnIdle { busy = false; failed = true }
        compose.onNodeWithTag("settings-relay-error").assertExists()
        compose.onNodeWithTag("settings-relay").performClick()
        compose.onNodeWithTag("settings-relay-custom").performClick()
        compose.onNodeWithTag("settings-custom-relay").performClick()
        compose.runOnIdle { assertEquals(2, customCalls); assertEquals(RelayPreference.Disabled, requested) }
        compose.runOnIdle { preference = RelayPreference.Disabled; failed = false }
        compose.onNodeWithTag("settings-relay").assertIsEnabled().assertTextContains("Direct connections only")
    }

    @Test fun explicitCustomRouteStartsWithExistingExactAddressAndNeedsApply() {
        val original = RelayPreference.Custom("https://relay.example.test/PUBLIC/path?Keep=Case")
        var applied: RelayPreference? = null
        compose.setContent { TaypeerTheme { RelaySettingsContent(original, false, false, { applied = it }, {}, {}, startCustom = true) } }
        compose.onNodeWithTag("relay-custom").assertIsSelected()
        compose.onNodeWithTag("relay-url").assertTextEquals(original.url)
        compose.runOnIdle { assertNull(applied) }
        compose.onNodeWithTag("apply-relay").performClick()
        compose.runOnIdle { assertEquals(original, applied) }
    }
}
