package dev.taypeer

import androidx.activity.compose.setContent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import dev.taypeer.qr.QrDecoder
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter

class InvitationUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private fun MainActivity.setContentForInvitationTest(content: @Composable () -> Unit) {
        runOnUiThread { setContent { TaypeerTheme { content() } } }
    }

    @Test fun manualCodeRequiresExplicitConnectionAndCancelDoesNotSubmit() {
        var joined: String? = null
        var cancelled = false
        var password: String? = null
        compose.activity.setContentForInvitationTest { ReceiveInvitationScreen({ code, secret -> joined = code; password = secret }, { cancelled = true }) }
        compose.onNodeWithTag("receive-connect").assertIsNotEnabled()
        val code = "PUBLIC synthetic code with trailing spaces  "
        compose.onNodeWithTag("invitation-code").performTextReplacement(code)
        compose.onNodeWithTag("receive-connect").assertIsNotEnabled()
        compose.onNodeWithTag("join-password").performTextReplacement("PUBLIC exact password  ")
        compose.runOnIdle { assertNull(joined) }
        compose.onNodeWithTag("receive-connect").performClick()
        compose.runOnIdle { assertEquals(code, joined); assertEquals("PUBLIC exact password  ", password); joined = null }
        compose.onNodeWithTag("invitation-code").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText, AnnotatedString("")))
        compose.onNodeWithTag("invitation-code").performTextReplacement(code)
        compose.onNodeWithTag("receive-cancel").performClick()
        compose.runOnIdle { assertTrue(cancelled); assertNull(joined) }
    }

    @Test fun backgroundClearsInvitationAndDoesNotConnect() {
        var joined = false
        compose.activity.setContentForInvitationTest { ReceiveInvitationScreen({ _, _ -> joined = true }, {}) }
        compose.onNodeWithTag("invitation-code").performTextReplacement("PUBLIC ephemeral bearer")
        compose.onNodeWithTag("join-password").performTextReplacement("PUBLIC ephemeral password")
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.onNodeWithTag("receive-connect").assertIsNotEnabled()
        compose.runOnIdle { assertFalse(joined) }
    }

    @Test fun decodedCameraCodePopulatesFormAndStillRequiresExplicitConnection() {
        var joined: String? = null
        val code = "PUBLIC scanned invitation"
        val matrix = QRCodeWriter().encode(code, BarcodeFormat.QR_CODE, 320, 320)
        val frame = ByteArray(320 * 320) { if (matrix[it % 320, it / 320]) 0 else 255.toByte() }
        compose.activity.setContentForInvitationTest {
            ReceiveInvitationScreen({ code, _ -> joined = code }, {}, scanner = { deliver, _ ->
                LaunchedEffect(Unit) { deliver(QrDecoder().decode(frame, 320, 320)!!) }
            })
        }
        compose.onNodeWithTag("scan-invitation").performClick()
        compose.onNodeWithTag("invitation-code").assertTextEquals(code)
        compose.runOnIdle { assertNull(joined) }
        compose.onNodeWithTag("join-password").performTextReplacement("PUBLIC scanned password")
        compose.onNodeWithTag("receive-connect").performClick()
        compose.runOnIdle { assertEquals(code, joined) }
    }
}
