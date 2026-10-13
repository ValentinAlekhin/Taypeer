package dev.taypeer

import android.Manifest
import android.accessibilityservice.AccessibilityServiceInfo
import android.content.pm.PackageManager
import android.view.accessibility.AccessibilityNodeInfo
import androidx.activity.compose.setContent
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.qr.QrScannerScreen
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Separate scenario: revoke CAMERA with adb before instrumentation, since revocation kills the UID. */
class QrPermissionScenario {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    private fun denyButton(node: AccessibilityNodeInfo?): AccessibilityNodeInfo? {
        if (node == null) return null
        if (node.viewIdResourceName?.endsWith(":id/permission_deny_button") == true) return node
        for (index in 0 until node.childCount) denyButton(node.getChild(index))?.let { return it }
        return null
    }

    @Test fun denyingSystemCameraPermissionKeepsManualEntryAvailable() {
        if (InstrumentationRegistry.getArguments().getString("qaResetCameraPermissionFlags") == "true") {
            resetCameraPermissionUserFlags()
        }
        assertEquals(PackageManager.PERMISSION_DENIED, compose.activity.checkSelfPermission(Manifest.permission.CAMERA))
        var closed = false
        var scanned = false
        compose.activity.runOnUiThread {
            compose.activity.setContent { TaypeerTheme { QrScannerScreen({ scanned = true }, { closed = true }) } }
        }
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        automation.serviceInfo = automation.serviceInfo.apply { flags = flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS }
        compose.waitUntil(20_000) { denyButton(automation.rootInActiveWindow)?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true }
        compose.onNodeWithTag("qr-camera-status").assertExists()
        compose.onNodeWithTag("qr-camera-preview").assertDoesNotExist()
        compose.onNodeWithTag("enter-qr-code").performClick()
        compose.runOnIdle { assertTrue(closed); assertFalse(scanned) }
        assertEquals(PackageManager.PERMISSION_DENIED, compose.activity.checkSelfPermission(Manifest.permission.CAMERA))
    }
}
