package dev.taypeer

import android.content.ClipData
import android.content.ClipboardManager
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.platform.SensitiveClipboard
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class ClipboardTest {
    @Test fun uncertainFailureBeforePublicationKeepsLegacyOriginalDeadlineAndRejectsAThirdOwner() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val system = context.getSystemService(ClipboardManager::class.java)
        val receipts = context.getSharedPreferences("clipboard-ownership", android.content.Context.MODE_PRIVATE)
        ActivityScenario.launch(MainActivity::class.java).use {
            instrumentation.waitForIdleSync()
            val cleared = CountDownLatch(1)
            val listener = ClipboardManager.OnPrimaryClipChangedListener { if (!system.hasPrimaryClip()) cleared.countDown() }
            instrumentation.runOnMainSync {
                check(receipts.edit().clear().commit())
                var publicationCalls = 0
                var failBeforePublication = false
                val tracked = SensitiveClipboard(context) { manager, clip ->
                    publicationCalls++
                    if (failBeforePublication) throw IllegalStateException("PUBLIC uncertain before publication")
                    manager.setPrimaryClip(clip)
                }
                tracked.focusChanged(true)
                tracked.copy("PUBLIC original uncertain clipboard value", 1u)
                val token = receipts.getString("token", null)
                val deadline = receipts.getLong("deadline", 0)
                // Older receipts had no timed flag; migration must preserve their deadline.
                check(receipts.edit().remove("timed").commit())
                failBeforePublication = true
                assertThrows(IllegalStateException::class.java) {
                    tracked.copy("PUBLIC unpublished no-clean clipboard value", null)
                }
                assertEquals("PUBLIC original uncertain clipboard value", system.primaryClip?.getItemAt(0)?.text?.toString())
                assertEquals(token, receipts.getString("token", null))
                assertEquals(deadline, receipts.getLong("deadline", 0))
                assertTrue(receipts.getBoolean("timed", false))
                assertNotNull(receipts.getString("candidate.token", null))
                assertFalse(receipts.getBoolean("candidate.timed", true))
                assertFalse(receipts.all.values.contains("PUBLIC original uncertain clipboard value"))
                assertFalse(receipts.all.values.contains("PUBLIC unpublished no-clean clipboard value"))

                tracked.focusChanged(false)
                val retained = receipts.all
                assertThrows(IllegalStateException::class.java) { tracked.copy("PUBLIC rejected third clipboard owner", 10u) }
                assertEquals(2, publicationCalls)
                assertEquals(retained, receipts.all)
                system.addPrimaryClipChangedListener(listener)
                tracked.focusChanged(true) // Reconcile the actual old owner, preserving its original timer.
                assertNull(receipts.getString("candidate.token", null))
            }
            try { assertTrue("The original owner's timer was lost", cleared.await(3, TimeUnit.SECONDS)) }
            finally { instrumentation.runOnMainSync { system.removePrimaryClipChangedListener(listener) } }
            instrumentation.runOnMainSync {
                assertFalse(system.hasPrimaryClip())
                assertTrue(receipts.all.isEmpty())
            }
        }
    }

    @Test fun uncertainFailureAfterFirstPublicationStillSchedulesTheCandidateDeadline() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val system = context.getSystemService(ClipboardManager::class.java)
        val receipts = context.getSharedPreferences("clipboard-ownership", android.content.Context.MODE_PRIVATE)
        ActivityScenario.launch(MainActivity::class.java).use {
            instrumentation.waitForIdleSync()
            val cleared = CountDownLatch(1)
            val listener = ClipboardManager.OnPrimaryClipChangedListener { if (!system.hasPrimaryClip()) cleared.countDown() }
            instrumentation.runOnMainSync {
                check(receipts.edit().clear().commit())
                val failed = SensitiveClipboard(context) { manager, clip ->
                    manager.setPrimaryClip(clip)
                    throw IllegalStateException("PUBLIC uncertain after publication")
                }
                failed.focusChanged(true)
                assertThrows(IllegalStateException::class.java) { failed.copy("PUBLIC published uncertain timed value", 1u) }
                assertEquals("PUBLIC published uncertain timed value", system.primaryClip?.getItemAt(0)?.text?.toString())
                assertNotNull(receipts.getString("token", null))
                assertTrue(receipts.getBoolean("timed", false))
                assertFalse(receipts.all.values.contains("PUBLIC published uncertain timed value"))
                system.addPrimaryClipChangedListener(listener)
            }
            // No focus callback rescues this timer: the exception path must schedule it itself.
            try { assertTrue("The published candidate's timer was not scheduled", cleared.await(3, TimeUnit.SECONDS)) }
            finally { instrumentation.runOnMainSync { system.removePrimaryClipChangedListener(listener) } }
            instrumentation.runOnMainSync {
                assertFalse(system.hasPrimaryClip())
                assertTrue(receipts.all.isEmpty())
            }
        }
    }

    @Test fun uncertainNoCleanPublicationSurvivesRecreationAndAnExpiredPreviousOwner() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val system = context.getSystemService(ClipboardManager::class.java)
        val receipts = context.getSharedPreferences("clipboard-ownership", android.content.Context.MODE_PRIVATE)
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            instrumentation.waitForIdleSync()
            instrumentation.runOnMainSync {
                check(receipts.edit().clear().commit())
                SensitiveClipboard(context).copy("PUBLIC expired prior clipboard value", 1u)
                check(receipts.edit().remove("timed").putLong("duration", 0).putLong("deadline", 0).commit())
                val failed = SensitiveClipboard(context) { manager, clip ->
                    manager.setPrimaryClip(clip)
                    throw IllegalStateException("PUBLIC uncertain no-clean publication")
                }
                failed.focusChanged(false)
                assertThrows(IllegalStateException::class.java) { failed.copy("PUBLIC published uncertain no-clean value", null) }
                assertNotNull(receipts.getString("candidate.token", null))
                assertFalse(receipts.getBoolean("candidate.timed", true))
            }
            activity.recreate()
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).focusChanged(true)
                assertEquals("PUBLIC published uncertain no-clean value", system.primaryClip?.getItemAt(0)?.text?.toString())
                assertFalse(receipts.getBoolean("timed", true))
                assertNull(receipts.getString("candidate.token", null))
                system.setPrimaryClip(ClipData.newPlainText("PUBLIC foreign clipboard", "PUBLIC foreign uncertain replacement"))
                SensitiveClipboard(context).focusChanged(true)
                assertEquals("PUBLIC foreign uncertain replacement", system.primaryClip?.getItemAt(0)?.text?.toString())
                assertTrue(receipts.all.isEmpty())
                system.clearPrimaryClip()
            }
        }
    }

    @Test fun definitePublicationDenialPreservesThePreviousReceiptAndDeadline() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val system = context.getSystemService(ClipboardManager::class.java)
        ActivityScenario.launch(MainActivity::class.java).use {
            instrumentation.waitForIdleSync()
            lateinit var denied: SensitiveClipboard
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).copy("PUBLIC original tracked value", 1u)
                denied = SensitiveClipboard(context) { _, _ -> throw SecurityException("PUBLIC copy access denied") }
                denied.focusChanged(true)
                assertThrows(SecurityException::class.java) { denied.copy("PUBLIC denied replacement", null) }
                assertEquals("PUBLIC original tracked value", system.primaryClip?.getItemAt(0)?.text?.toString())
            }
            android.os.SystemClock.sleep(1200)
            instrumentation.runOnMainSync {
                denied.focusChanged(true)
                assertFalse(system.hasPrimaryClip())
            }
        }
    }
    @Test fun noCleanReplacesAnEarlierTimerAndSurvivesRecreation() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val system = context.getSystemService(ClipboardManager::class.java)
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            instrumentation.waitForIdleSync()
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).apply {
                    focusChanged(true)
                    copy("PUBLIC prior timed value", 1u)
                    copy("PUBLIC no clean value", null)
                }
            }
            android.os.SystemClock.sleep(1200)
            activity.recreate()
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).focusChanged(true)
                assertEquals("PUBLIC no clean value", system.primaryClip?.getItemAt(0)?.text?.toString())
                system.setPrimaryClip(ClipData.newPlainText("PUBLIC other app", "PUBLIC foreign no clean"))
                SensitiveClipboard(context).focusChanged(true)
                assertEquals("PUBLIC foreign no clean", system.primaryClip?.getItemAt(0)?.text?.toString())
                system.clearPrimaryClip()
            }
        }
    }
    @Test fun ownershipSurvivesRecreationAndForeignCopyIsPreserved() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val system = context.getSystemService(ClipboardManager::class.java)
        ActivityScenario.launch(MainActivity::class.java).use {
            instrumentation.waitForIdleSync()
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).apply {
                    copy("PUBLIC-owned-value", 1u)
                    focusChanged(false)
                }
            }
            android.os.SystemClock.sleep(1200)
            instrumentation.runOnMainSync {
                SensitiveClipboard(context).focusChanged(true)
                assertFalse(system.hasPrimaryClip())
                SensitiveClipboard(context).apply {
                    copy("PUBLIC-second-value", 1u)
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
