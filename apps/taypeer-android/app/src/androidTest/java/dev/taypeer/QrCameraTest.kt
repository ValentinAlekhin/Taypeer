package dev.taypeer

import android.Manifest
import androidx.activity.compose.setContent
import androidx.camera.core.CameraState
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.compose.material3.Text
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.qr.QrScannerScreen
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.util.concurrent.TimeUnit

/** Real CameraX lifecycle on an ARM64 emulator, with no image fixture written to camera storage. */
class QrCameraTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    @Test fun virtualCameraOpensAndReleasesWhenScannerCloses() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(compose.activity.packageName, Manifest.permission.CAMERA)
        val provider = ProcessCameraProvider.getInstance(compose.activity).get(15, TimeUnit.SECONDS)
        var close = false
        var scanned = false
        compose.activity.runOnUiThread {
            compose.activity.setContent { TaypeerTheme { QrScannerScreen({ scanned = true }, { close = true }) } }
        }
        compose.onNodeWithTag("qr-camera-preview").assertExists()
        compose.waitUntil(20_000) { provider.availableCameraInfos.any { it.cameraState.value?.type == CameraState.Type.OPEN } }
        compose.onNodeWithTag("enter-qr-code").performClick()
        compose.runOnIdle { assertTrue(close); assertFalse(scanned) }
        compose.activity.runOnUiThread { compose.activity.setContent { Text("PUBLIC scanner closed") } }
        compose.waitUntil(20_000) { provider.availableCameraInfos.all { it.cameraState.value?.type == CameraState.Type.CLOSED } }
    }
}
