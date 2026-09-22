package dev.taypeer

import android.content.ClipData
import android.content.ClipboardManager
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.SensitiveClipboard
import org.junit.Assert.*
import org.junit.Test

class ClipboardTest {
    @Test fun ownershipSurvivesRecreationAndForeignCopyIsPreserved() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val system = context.getSystemService(ClipboardManager::class.java)
        ActivityScenario.launch(MainActivity::class.java).use {
            instrumentation.waitForIdleSync()
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).apply {
                    copy("PUBLIC-owned-value", 1)
                    focusChanged(false)
                }
            }
            android.os.SystemClock.sleep(1200)
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).focusChanged(true)
                assertFalse(system.hasPrimaryClip())
                SensitiveClipboard(context).apply {
                    copy("PUBLIC-second-value", 1)
                    focusChanged(false)
                }
                system.setPrimaryClip(ClipData.newPlainText("PUBLIC-other-app", "PUBLIC-foreign-value"))
            }
            android.os.SystemClock.sleep(1200)
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).focusChanged(true)
                assertEquals("PUBLIC-foreign-value", system.primaryClip?.getItemAt(0)?.text?.toString())
                system.clearPrimaryClip()
            }
        }
    }
}
