package dev.taypeer

import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.AndroidComposeTestRule

/** Wait for the real IME layout, then verify that the physical Unlock tap was accepted. */
internal fun AndroidComposeTestRule<*, MainActivity>.tapWorkingCopyUnlock(expectedDatabase: String) {
    runOnIdle {
        activity.getSystemService(InputMethodManager::class.java)
            .hideSoftInputFromWindow(activity.window.decorView.windowToken, 0)
    }
    waitUntil(10_000) {
        activity.window.decorView.rootWindowInsets?.let { !it.isVisible(WindowInsets.Type.ime()) } == true
    }
    onNodeWithTag("authenticate").assertIsEnabled().assertIsDisplayed().performTouchInput { click() }
    try {
        waitUntil(20_000) {
            val state = activity.documentState
            state.opening || state.overview?.database == expectedDatabase || state.error
        }
    } catch (timeout: ComposeTimeoutException) {
        var flags = ""
        runOnIdle {
            val state = activity.documentState
            flags = "opening=${state.opening}, overview=${state.overview != null}, expectedDatabase=${state.overview?.database == expectedDatabase}, navigating=${state.navigating}, failure=${state.error}, secrecyEpoch=${state.secrecyEpoch}, ime=${activity.window.decorView.rootWindowInsets?.isVisible(WindowInsets.Type.ime())}"
        }
        throw AssertionError("Physical Unlock tap was not acknowledged; $flags", timeout)
    }
}
