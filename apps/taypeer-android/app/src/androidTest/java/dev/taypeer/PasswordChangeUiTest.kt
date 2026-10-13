package dev.taypeer

import androidx.activity.OnBackPressedDispatcher
import androidx.activity.compose.LocalOnBackPressedDispatcherOwner
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** PUBLIC synthetic passwords only; native rotation validation is covered by worker tests. */
class PasswordChangeUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun mismatchedRepeatBlocksSubmitAndMatchingInputKeepsSignificantSpaces() {
        val password = " PUBLIC rotation password with spaces "
        var submitted: String? = null
        var closes = 0
        compose.setContent {
            TaypeerTheme {
                PasswordChangeScreen(false, false, false, { submitted = it }, { closes++ })
            }
        }
        compose.onNodeWithTag("apply-password-change").assertIsNotEnabled()
        compose.onNodeWithTag("rotation-new-password").performTextReplacement(password)
        compose.onNodeWithTag("rotation-repeat-password").performTextReplacement(password.trim())
        compose.onNodeWithTag("rotation-password-mismatch").assertExists()
        compose.onNodeWithTag("apply-password-change").assertIsNotEnabled()
        compose.runOnIdle { assertNull(submitted) }
        compose.onNodeWithTag("rotation-repeat-password").performTextReplacement(password)
        compose.onNodeWithTag("rotation-password-mismatch").assertDoesNotExist()
        compose.onNodeWithTag("apply-password-change").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(password, submitted); assertEquals(0, closes) }
    }

    @Test fun nativeFailureRetainsBothFieldsAndDoesNotCloseTheForm() {
        val password = "PUBLIC failed rotation input"
        var busy by mutableStateOf(false)
        var error by mutableStateOf(false)
        var calls = 0
        var closes = 0
        compose.setContent {
            TaypeerTheme {
                PasswordChangeScreen(false, busy, error, { calls++; busy = true }, { closes++ })
            }
        }
        fill(password)
        compose.onNodeWithTag("apply-password-change").performClick()
        compose.onNodeWithTag("password-change-busy").assertExists()
        compose.runOnIdle { busy = false; error = true }
        compose.onNodeWithTag("password-change-error").assertExists()
        compose.onNodeWithTag("password-change-screen").assertExists()
        revealBoth()
        compose.onNodeWithTag("rotation-new-password").assertTextEquals(password)
        compose.onNodeWithTag("rotation-repeat-password").assertTextEquals(password)
        compose.onNodeWithTag("apply-password-change").assertIsEnabled()
        compose.runOnIdle { assertEquals(1, calls); assertEquals(0, closes) }
        compose.onNodeWithTag("cancel-form").performClick()
        compose.runOnIdle { assertEquals(1, closes) }
    }

    @Test fun retainedUnknownResultDisablesEditingAndRetriesTheExactSubmittedPassword() {
        val password = " PUBLIC retained revoke password "
        var busy by mutableStateOf(false)
        var error by mutableStateOf(false)
        var retained by mutableStateOf(false)
        val submissions = mutableListOf<String>()
        var closes = 0
        compose.setContent {
            TaypeerTheme {
                PasswordChangeScreen(true, busy, error, {
                    submissions += it; retained = true; busy = true
                }, { closes++ }, retained)
            }
        }
        fill(password)
        compose.onNodeWithTag("apply-password-change").performClick()
        compose.runOnIdle { busy = false; error = true }
        compose.onNodeWithTag("password-change-error").assertExists()
        compose.onNodeWithTag("rotation-new-password").assertIsNotEnabled()
        compose.onNodeWithTag("rotation-repeat-password").assertIsNotEnabled()
        compose.onNodeWithTag("reveal-rotation-password").assertIsNotEnabled()
        compose.onNodeWithTag("reveal-rotation-repeat").assertIsNotEnabled()
        compose.onNodeWithTag("apply-password-change").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(listOf(password, password), submissions); assertEquals(0, closes) }
        compose.runOnIdle { busy = false }
        compose.onNodeWithTag("cancel-form").assertIsNotEnabled()
        compose.onNodeWithTag("navigation-back").performClick()
        compose.runOnIdle { assertEquals(0, closes) }
        compose.onNodeWithTag("password-change-screen").assertExists()
    }

    @Test fun revealControlsAreIndependentAndBusyConsumesSystemBackAndBlocksDuplicateApply() {
        val password = "PUBLIC busy rotation input"
        var busy by mutableStateOf(false)
        var calls = 0
        var closes = 0
        var dispatcher: OnBackPressedDispatcher? = null
        compose.setContent {
            val owner = checkNotNull(LocalOnBackPressedDispatcherOwner.current)
            SideEffect { dispatcher = owner.onBackPressedDispatcher }
            TaypeerTheme {
                PasswordChangeScreen(false, busy, false, { calls++; busy = true }, { closes++ })
            }
        }
        fill(password)
        compose.onNodeWithTag("rotation-new-password").assertMaskedText(password.length)
        compose.onNodeWithTag("rotation-repeat-password").assertMaskedText(password.length)
        compose.onNodeWithTag("reveal-rotation-password").performScrollTo().performClick()
        compose.onNodeWithTag("rotation-new-password").assertRenderedText(password)
        compose.onNodeWithTag("rotation-repeat-password").assertMaskedText(password.length)
        compose.onNodeWithTag("reveal-rotation-repeat").performScrollTo().performClick()
        compose.onNodeWithTag("rotation-repeat-password").assertRenderedText(password)
        compose.onNodeWithTag("apply-password-change").performClick()
        compose.onNodeWithTag("apply-password-change").assertIsNotEnabled()
        compose.onNodeWithTag("cancel-form").assertIsNotEnabled()
        compose.onNodeWithTag("rotation-new-password").assertIsNotEnabled().assertMaskedText(password.length)
        compose.onNodeWithTag("rotation-repeat-password").assertIsNotEnabled().assertMaskedText(password.length)
        compose.onNodeWithTag("reveal-rotation-password").assertIsNotEnabled()
        compose.onNodeWithTag("navigation-back").performClick()
        compose.runOnIdle { checkNotNull(dispatcher).onBackPressed() }
        compose.runOnIdle { assertEquals(1, calls); assertEquals(0, closes) }
        compose.onNodeWithTag("password-change-screen").assertExists()
        compose.runOnIdle { busy = false }
        compose.runOnIdle { checkNotNull(dispatcher).onBackPressed() }
        compose.runOnIdle { assertEquals(1, closes) }
    }

    private fun fill(password: String) {
        compose.onNodeWithTag("rotation-new-password").performTextReplacement(password)
        compose.onNodeWithTag("rotation-repeat-password").performTextReplacement(password)
    }

    private fun revealBoth() {
        compose.onNodeWithTag("reveal-rotation-password").performScrollTo().performClick()
        compose.onNodeWithTag("reveal-rotation-repeat").performScrollTo().performClick()
    }
}
