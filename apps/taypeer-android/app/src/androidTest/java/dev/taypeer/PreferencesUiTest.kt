package dev.taypeer

import androidx.activity.compose.setContent
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.input.key.Key
import androidx.compose.foundation.layout.Column
import androidx.lifecycle.Lifecycle
import dev.taypeer.bridge.DeviceRow
import dev.taypeer.bridge.DeviceSettingsView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class PreferencesUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private lateinit var originalTheme: ThemeMode
    private lateinit var originalLanguage: UiLanguage
    private lateinit var originalDeviceSettings: DeviceSettingsView
    private val application get() = compose.activity.application as TaypeerApplication
    private val preferences get() = (compose.activity.application as TaypeerApplication).uiPreferences

    @Before fun establishPublicPreferences() {
        originalDeviceSettings = runBlocking { withContext(Dispatchers.IO) { application.host.await().deviceSettings() } }
        compose.runOnIdle {
            originalTheme = preferences.theme; originalLanguage = preferences.language
            preferences.selectTheme(ThemeMode.SYSTEM); preferences.selectLanguage(UiLanguage.ENGLISH)
        }
    }

    @After fun restorePreferences() {
        if (::originalDeviceSettings.isInitialized) compose.waitUntil(20_000) { !application.deviceSettings.busy }
        if (::originalDeviceSettings.isInitialized) runBlocking {
            withContext(Dispatchers.IO) {
                application.host.await().apply {
                    setDeviceName(originalDeviceSettings.deviceName)
                    setIdleSeconds(originalDeviceSettings.idleSeconds)
                    setClipboardSeconds(originalDeviceSettings.clipboardSeconds)
                }
            }
        }
        compose.runOnIdle {
            if (::originalTheme.isInitialized) preferences.selectTheme(originalTheme)
            if (::originalLanguage.isInitialized) preferences.selectLanguage(originalLanguage)
        }
        if (::originalDeviceSettings.isInitialized) {
            compose.runOnIdle { application.deviceSettings.editName(originalDeviceSettings.deviceName); application.deviceSettings.load() }
            compose.waitUntil(20_000) { !application.deviceSettings.busy && application.deviceSettings.settings == originalDeviceSettings }
        }
    }

    @Test fun themeAndLanguageAreAvailableBeforeUnlockAndPersistAcrossRecreation() {
        compose.onNodeWithTag("create-database").assertExists()
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-settings").performClick()
        compose.onNodeWithTag("theme-selector").performScrollTo().performClick()
        compose.onNodeWithTag("theme-dark").performClick()
        compose.onNodeWithTag("theme-selector").assertTextContains("Dark")
        compose.onNodeWithTag("language-selector").performScrollTo().performClick()
        compose.onNodeWithTag("language-ru").performClick()
        compose.onNodeWithText("Настройки").assertExists()
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-settings").performClick()
        compose.onNodeWithTag("theme-selector").performScrollTo().assertTextContains("Тёмная")
        compose.onNodeWithTag("language-selector").performScrollTo().assertTextContains("Русский")
        compose.runOnIdle {
            val reloaded = UiPreferences(compose.activity)
            assertEquals(ThemeMode.DARK, reloaded.theme)
            assertEquals(UiLanguage.RUSSIAN, reloaded.language)
            assertNull(compose.activity.documentState.overview)
        }
    }

    @Test fun generatorCharacterSetsAreIndependentAndModeKeepsTypedOptions() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-generator").performClick()
        compose.onNodeWithTag("generator-uppercase").assertIsOn().performClick().assertIsOff()
        compose.onNodeWithTag("generator-lowercase").assertIsOn()
        compose.onNodeWithTag("generator-digits").assertIsOn()
        compose.onNodeWithTag("length").performTextReplacement("44")
        compose.onNodeWithTag("generator-mode").performClick()
        compose.onNodeWithTag("generator-passphrase-mode").performClick()
        compose.onNodeWithTag("generator-words").performTextReplacement("8")
        compose.onNodeWithTag("generator-mode").performClick()
        compose.onNodeWithTag("generator-password-mode").performClick()
        compose.onNodeWithTag("length").assertTextContains("44")
        compose.onNodeWithTag("generator-uppercase").assertIsOff()
        compose.onNodeWithTag("generator-mode").performClick()
        compose.onNodeWithTag("generator-passphrase-mode").performClick()
        compose.onNodeWithTag("generator-words").assertTextContains("8")
    }

    @Test fun deviceDurationsPersistAndNativeRejectionPreservesTypedInput() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-settings").performClick()
        compose.waitUntil(20_000) { application.deviceSettings.settings != null && !application.deviceSettings.busy }
        compose.onNodeWithTag("idle-selector").performScrollTo().performClick()
        compose.onNodeWithTag("idle-60").performClick()
        compose.waitUntil(20_000) { application.deviceSettings.settings?.idleSeconds == 60u && !application.deviceSettings.busy }
        compose.onNodeWithTag("clipboard-selector").performScrollTo().performClick()
        compose.onNodeWithTag("clipboard-never").performClick()
        compose.waitUntil(20_000) { application.deviceSettings.settings?.clipboardSeconds == null && !application.deviceSettings.busy }
        compose.onNodeWithTag("clipboard-selector").assertTextContains("Do not clear")
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-settings").performClick()
        compose.onNodeWithTag("idle-selector").performScrollTo().assertTextContains("After 1 minute")
        compose.onNodeWithTag("clipboard-selector").performScrollTo().assertTextContains("Do not clear")
        val persisted = runBlocking { withContext(Dispatchers.IO) { application.host.await().deviceSettings() } }
        assertEquals(DeviceSettingsView(60u, null, originalDeviceSettings.deviceName), persisted)

        compose.onNodeWithTag("idle-selector").performScrollTo().performClick()
        compose.onNodeWithTag("idle-custom").performClick()
        compose.onNodeWithTag("duration-seconds").performTextReplacement("0")
        compose.onNodeWithTag("apply-duration").assertIsEnabled().performClick()
        compose.waitUntil(20_000) { application.deviceSettings.failed && !application.deviceSettings.busy }
        compose.onNodeWithTag("duration-dialog").assertExists()
        compose.onNodeWithTag("duration-seconds").assertTextContains("0")
        compose.onNodeWithTag("device-settings-error").assertExists()
        assertEquals(persisted, runBlocking { withContext(Dispatchers.IO) { application.host.await().deviceSettings() } })
        compose.onNodeWithTag("cancel-duration").performClick()
        compose.onNodeWithTag("idle-selector").assertTextContains("After 1 minute")
    }

    @Test fun systemBackFromSettingsReturnsToTheOpenDatabase() {
        val document = PublicDocumentUiFixture(compose)
        document.create()
        var database = ""
        compose.runOnIdle { database = compose.activity.documentState.overview!!.database }
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-settings").performClick()
        compose.onNodeWithTag("route-settings").assertExists()
        compose.runOnIdle { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.waitUntil(20_000) { compose.onAllNodes(hasTestTag("route-entries")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("master-password").assertDoesNotExist()
        compose.runOnIdle {
            assertEquals(database, compose.activity.documentState.overview?.database)
            assertFalse(compose.activity.isFinishing)
            assertEquals(Lifecycle.State.RESUMED, compose.activity.lifecycle.currentState)
        }
    }

    @OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
    @Test fun deviceNameAppliesOnImeEnterAndBlurAndNativeRejectionRetainsRawInput() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-settings").performClick()
        compose.waitUntil(20_000) { application.deviceSettings.settings != null && !application.deviceSettings.busy }
        compose.onNodeWithTag("device-name").performScrollTo().assertTextEquals(originalDeviceSettings.deviceName)
        val raw = "  PUBLIC Android телефон α  "
        compose.onNodeWithTag("device-name").performTextReplacement(raw)
        assertEquals(originalDeviceSettings.deviceName, nativeDeviceSettings().deviceName)
        compose.onNodeWithTag("device-name").performImeAction()
        awaitName(raw)
        compose.onNodeWithTag("device-name").assertTextEquals(raw)
        assertEquals(raw, nativeDeviceSettings().deviceName)

        val blurred = "PUBLIC name submitted by keyboard focus change"
        compose.onNodeWithTag("device-name").performTextReplacement(blurred)
        compose.onNodeWithTag("device-name").performKeyInput { pressKey(Key.Tab) }
        awaitName(blurred)
        assertEquals(blurred, nativeDeviceSettings().deviceName)

        val entered = "PUBLIC name submitted by hardware Enter"
        compose.onNodeWithTag("device-name").performTextReplacement(entered)
        compose.onNodeWithTag("device-name").performKeyInput { pressKey(Key.Enter) }
        awaitName(entered)
        assertEquals(entered, nativeDeviceSettings().deviceName)

        compose.onNodeWithTag("device-name").performTextReplacement("   ")
        compose.onNodeWithTag("device-name").performImeAction()
        compose.waitUntil(20_000) { !application.deviceSettings.busy && application.deviceSettings.nameFailure == DeviceNameFailure.REQUIRED }
        compose.onNodeWithTag("device-name").assertTextEquals("   ")
        compose.onNodeWithTag("device-name-error").assertTextEquals("Enter a device name.")
        assertEquals(entered, nativeDeviceSettings().deviceName)

        val tooLong = "PUBLIC " + "Ж".repeat(125)
        compose.onNodeWithTag("device-name").performTextReplacement(tooLong)
        compose.onNodeWithTag("device-name").performImeAction()
        compose.waitUntil(20_000) { !application.deviceSettings.busy && application.deviceSettings.nameFailure == DeviceNameFailure.TOO_LONG }
        compose.onNodeWithTag("device-name").assertTextEquals(tooLong)
        compose.onNodeWithTag("device-name-error").assertTextEquals("The device name must fit within 256 UTF-8 bytes.")
        assertEquals(entered, nativeDeviceSettings().deviceName)

        compose.onNodeWithTag("device-name").performTextReplacement(raw)
        compose.onNodeWithTag("device-name").performImeAction()
        awaitName(raw)
        compose.onNodeWithTag("device-name-error").assertDoesNotExist()
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-settings").performClick()
        compose.onNodeWithTag("device-name").performScrollTo().assertTextEquals(raw)
        assertEquals(raw, nativeDeviceSettings().deviceName)
    }

    private fun nativeDeviceSettings() = runBlocking { withContext(Dispatchers.IO) { application.host.await().deviceSettings() } }
    private fun awaitName(value: String) = compose.waitUntil(20_000) {
        !application.deviceSettings.busy && application.deviceSettings.settings?.deviceName == value &&
            application.deviceSettings.nameDraft == value && application.deviceSettings.nameFailure == null
    }

    @Test fun onlyLocalExchangeLabelUsesAcknowledgedNativeNameInsteadOfUnsubmittedDraft() {
        val local = DeviceRow("PUBLIC-local-device-identity", true, true, null, dev.taypeer.bridge.PeerAvailability.UNKNOWN, dev.taypeer.bridge.ExchangePhase.WAITING)
        val remote = DeviceRow("PUBLIC-remote-device-identity", false, false, null, dev.taypeer.bridge.PeerAvailability.UNKNOWN, dev.taypeer.bridge.ExchangePhase.WAITING)
        compose.waitUntil(20_000) { !application.deviceSettings.busy && application.deviceSettings.settings != null }
        compose.activityRule.scenario.onActivity { activity -> activity.setContent { TaypeerTheme { Column {
            ExchangeDeviceLabel(local, application.deviceSettings.settings?.deviceName)
            ExchangeDeviceLabel(remote, application.deviceSettings.settings?.deviceName)
        } } } }
        val draft = "PUBLIC unsubmitted local device label"
        compose.runOnIdle { application.deviceSettings.editName(draft) }
        compose.onNodeWithText(originalDeviceSettings.deviceName).assertExists()
        compose.onNodeWithText(draft).assertDoesNotExist()
        compose.onNodeWithText(remote.id.take(12)).assertExists()
        assertEquals(originalDeviceSettings.deviceName, nativeDeviceSettings().deviceName)
        compose.runOnIdle { application.deviceSettings.submitName() }
        awaitName(draft)
        compose.onNodeWithText(draft).assertExists()
        compose.onNodeWithText(local.id.take(12)).assertDoesNotExist()
        compose.onNodeWithText(remote.id.take(12)).assertExists()
        assertEquals(draft, nativeDeviceSettings().deviceName)
    }
}
