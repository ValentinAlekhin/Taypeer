package dev.taypeer

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.text.TextLayoutResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue

/** Compose 1.8 retains its Password semantic flag after reveal; verify the actual laid-out text. */
internal fun SemanticsNodeInteraction.assertRenderedText(expected: String): SemanticsNodeInteraction {
    val layouts = mutableListOf<TextLayoutResult>()
    performSemanticsAction(SemanticsActions.GetTextLayoutResult) { action ->
        assertTrue("The field must expose its actual text layout", action(layouts))
    }
    assertEquals("A field must have one actual text layout", 1, layouts.size)
    assertEquals(expected, layouts.single().layoutInput.text.text)
    return this
}

internal fun SemanticsNodeInteraction.assertMaskedText(length: Int): SemanticsNodeInteraction =
    assertRenderedText("\u2022".repeat(length))
