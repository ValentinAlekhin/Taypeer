package dev.taypeer

import android.Manifest
import android.content.pm.PackageManager
import android.os.Process
import android.os.UserHandle
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals

// PackageManager SystemApi constants are omitted from public SDK stubs.
private const val CAMERA_USER_SET = 1 shl 0
private const val CAMERA_USER_FIXED = 1 shl 1
private const val GRANT_RUNTIME_PERMISSIONS = "android.permission.GRANT_RUNTIME_PERMISSIONS"

/** API 31 lacks the pm flag-reset command; only this isolated denial scenario uses TestApi. */
internal fun resetCameraPermissionUserFlags() {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val context = instrumentation.targetContext
    val packageName = "dev.taypeer"
    assertEquals("Reset is scoped to Taypeer", packageName, context.packageName)
    val permission = Manifest.permission.CAMERA
    val mask = CAMERA_USER_SET or CAMERA_USER_FIXED
    val user = Process.myUserHandle()
    val update = PackageManager::class.java.getMethod(
        "updatePermissionFlags",
        String::class.java,
        String::class.java,
        Int::class.javaPrimitiveType,
        Int::class.javaPrimitiveType,
        UserHandle::class.java,
    )
    val read = PackageManager::class.java.getMethod(
        "getPermissionFlags",
        String::class.java,
        String::class.java,
        UserHandle::class.java,
    )
    val automation = instrumentation.uiAutomation
    automation.adoptShellPermissionIdentity(GRANT_RUNTIME_PERMISSIONS)
    try {
        update.invoke(context.packageManager, permission, packageName, mask, 0, user)
        val flags = read.invoke(context.packageManager, permission, packageName, user) as Int
        assertEquals("CAMERA user decision flags must be cleared", 0, flags and mask)
        assertEquals("Runner must revoke CAMERA before instrumentation", PackageManager.PERMISSION_DENIED, context.checkSelfPermission(permission))
    } finally {
        automation.dropShellPermissionIdentity()
    }
}
