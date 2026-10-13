package dev.taypeer

import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.lifecycle.Lifecycle
import org.junit.Rule
import org.junit.Test

class GeneratorTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    @Test fun configurationRecreationRetainsOnlyTheInMemoryForm() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-generator").performClick()
        compose.onNodeWithTag("length").performTextReplacement("44")
        compose.waitUntil(10_000) {
            compose.onAllNodes(androidx.compose.ui.test.hasTestTag("generated-password"))
                .fetchSemanticsNodes().isNotEmpty()
        }
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("length").assertTextContains("44")
        compose.onNodeWithTag("generated-password").assertExists()
    }
    @Test fun actualComposeToRustResultIsRemovedWhenActivityStops() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-generator").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(androidx.compose.ui.test.hasTestTag("generated-password"))
                .fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("generated-password").assertExists()
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.onNodeWithTag("generated-password").assertDoesNotExist()
    }

    @Test fun optionsGenerateThroughRustAndInvalidInputRevokesTheResult() {
        compose.onNodeWithTag("screen-actions").performClick()
        compose.onNodeWithTag("open-generator").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(androidx.compose.ui.test.hasTestTag("generated-password")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("length").performTextReplacement("8")
        compose.waitUntil(10_000) {
            compose.onAllNodes(androidx.compose.ui.test.hasTestTag("generator-quality")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("generator-quality").assertTextContains("8", substring = true)
        compose.onNodeWithTag("length").performTextReplacement("")
        compose.onNodeWithTag("generated-password").assertDoesNotExist()
        compose.onNodeWithTag("generator-error").assertExists()
        compose.onNodeWithTag("generator-copy").assertIsNotEnabled()
        compose.onNodeWithTag("generator-mode").performClick()
        compose.onNodeWithTag("generator-passphrase-mode").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(androidx.compose.ui.test.hasTestTag("generated-password")).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("generator-words").assertTextContains("6")
        compose.onNodeWithTag("generator-quality").assertTextContains("77", substring = true)
    }
}
