package dev.taypeer

import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.KeystoreCredentials
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

/** Real Native public workflow over authenticated Iroh and private isolated document workers. */
class ManagementNetworkTest {
    @Test fun twoExplicitDecisionsApplyOneTransferWithoutChangingPasswordRootOrHistory() {
        PublicManagementFixture().use { fixture ->
            fixture.prepare()
            val history = fixture.sourceSession.history(FormKind.ENTRY, fixture.entry)
            val row = fixture.propose()
            assertEquals(ManagementPhase.PROPOSED, row.phase)
            assertNull(row.applied)
            assertEquals(row.intent, fixture.peerHost.managementHandoffs(fixture.database).single().intent)
            val status = fixture.sourceHost.managementStatus(row.intent)
            assertEquals(row.intent, status.intent)
            val ready = status.readiness as ManagementReadinessView.Ready
            assertEquals(fixture.recipient, ready.device)
            assertEquals(row.intent.previous, ready.control)
            assertEquals(fixture.peerSession!!.generation(), ready.generation)
            val accepted = fixture.accept(row.intent)
            assertEquals(ManagementPhase.CONSENT_RECORDED, accepted.phase)
            assertNull(accepted.applied)
            assertTrue(fixture.sourceSession.overview("").managing)
            assertFalse(fixture.peerSession!!.overview("").managing)
            val completed = fixture.complete(row.intent)
            assertNotNull(completed.applied)
            assertFalse(fixture.sourceSession.overview("").managing)
            assertTrue(fixture.peerSession!!.overview("").managing)
            val refusal = assertThrows(AndroidException.Management::class.java) { fixture.sourceSession.proposeManagement(fixture.recipient, newOperation()) }
            assertEquals(ManagementFailure.WRONG_ROLE, refusal.failure)
            assertEquals(history, fixture.sourceSession.history(FormKind.ENTRY, fixture.entry))
            assertEquals(history, fixture.peerSession!!.history(FormKind.ENTRY, fixture.entry))
            assertEquals(fixture.savedPassword, fixture.peerSession!!.reveal(fixture.entry, null, null))
            assertEquals(fixture.sourceCopy.root, fixture.sourceDocuments.catalog().single().root)
            assertEquals(fixture.peerCopy.root, fixture.peerDocuments.catalog().single().root)
            fixture.stopNetwork()
            val source = File(fixture.sourceCopy.path).readBytes()
            val peer = File(fixture.peerCopy.path).readBytes()
            val repeated = fixture.sourceHost.retryManagement(row.intent)
            assertEquals(completed.applied!!.transition, repeated.applied!!.transition)
            assertArrayEquals(source, File(fixture.sourceCopy.path).readBytes())
            assertArrayEquals(peer, File(fixture.peerCopy.path).readBytes())
            fixture.closePeer()
            val reopened = fixture.reopenPeer()
            assertTrue(reopened.overview("").managing)
            assertEquals(history, reopened.history(FormKind.ENTRY, fixture.entry))
            assertEquals(fixture.savedPassword, reopened.reveal(fixture.entry, null, null))
        }
    }

    @Test fun explicitRejectionChangesNoArchiveAndLockedReadinessDoesNotReuseOldGeneration() {
        PublicManagementFixture().use { fixture ->
            fixture.prepare()
            val first = fixture.propose()
            val beforeSource = File(fixture.sourceCopy.path).readBytes()
            val beforePeer = File(fixture.peerCopy.path).readBytes()
            val rejected = fixture.peerSession!!.rejectManagement(first.intent)
            assertEquals(ManagementPhase.REJECTED, rejected.phase)
            assertEquals(ManagementPhase.REJECTED, fixture.sourceHost.retryManagement(first.intent).phase)
            assertEquals(ManagementPhase.REJECTED, fixture.peerSession!!.rejectManagement(first.intent).phase)
            assertArrayEquals(beforeSource, File(fixture.sourceCopy.path).readBytes())
            assertArrayEquals(beforePeer, File(fixture.peerCopy.path).readBytes())
            val next = fixture.propose()
            val ready = fixture.sourceHost.managementStatus(next.intent).readiness as ManagementReadinessView.Ready
            fixture.closePeer()
            assertEquals(ManagementReadinessView.Locked, fixture.sourceHost.managementStatus(next.intent).readiness)
            assertTrue(fixture.sourceSession.overview("").managing)
            assertNull(fixture.sourceHost.managementHandoffs(fixture.database).single { it.intent == next.intent }.applied)
            val reopened = fixture.reopenPeer()
            val fresh = fixture.sourceHost.managementStatus(next.intent).readiness as ManagementReadinessView.Ready
            assertEquals(reopened.generation(), fresh.generation)
            assertNotEquals(ready.generation, fresh.generation)
            assertEquals(next.intent, fixture.peerHost.managementHandoffs(fixture.database).single { it.intent.operation == next.intent.operation }.intent)
            fixture.accept(next.intent)
            fixture.complete(next.intent)
            assertEquals(2, fixture.sourceHost.managementHandoffs(fixture.database).size)
        }
    }

    @Test fun lostAcceptRequestedPublicationSurvivesHostRestartAndCannotBecomeARejection() {
        val armed = AtomicBoolean()
        val fired = AtomicBoolean()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        PublicManagementFixture { index ->
            val delegate = KeystoreCredentials(context)
            if (index == 0) delegate else object : CredentialPort by delegate {
                override fun set(service: String, account: String, bytes: ByteArray) {
                    // The real store erases its input on return; retain only this PUBLIC phase match.
                    val acceptRequested = account.startsWith("state:management:") && armed.get() &&
                        bytes.toString(Charsets.UTF_8).contains("\"kind\":\"accept_requested\"")
                    delegate.set(service, account, bytes)
                    // This synthetic metadata-only fault targets the actual private journal phase.
                    if (acceptRequested && fired.compareAndSet(false, true))
                        throw AndroidException.CommitUncertain()
                }
            }
        }.use { fixture ->
            fixture.prepare()
            val proposal = fixture.propose()
            val history = fixture.sourceSession.history(FormKind.ENTRY, fixture.entry)
            val sourceBefore = File(fixture.sourceCopy.path).readBytes()
            armed.set(true)
            assertThrows(AndroidException.CommitUncertain::class.java) { fixture.peerSession!!.acceptManagement(proposal.intent) }
            assertTrue(fired.get())
            fixture.restartPeer()
            val retained = fixture.peerHost.managementHandoffs(fixture.database).single()
            assertEquals(proposal.intent, retained.intent)
            assertEquals(ManagementPhase.ACCEPT_REQUESTED, retained.phase)
            val refusal = assertThrows(AndroidException.Management::class.java) { fixture.peerSession!!.rejectManagement(proposal.intent) }
            assertEquals(ManagementFailure.CONFLICT, refusal.failure)
            assertArrayEquals(sourceBefore, File(fixture.sourceCopy.path).readBytes())
            val consent = fixture.peerHost.retryManagement(proposal.intent)
            assertEquals(ManagementPhase.CONSENT_RECORDED, consent.phase)
            fixture.complete(proposal.intent)
            assertEquals(1, fixture.sourceHost.managementHandoffs(fixture.database).size)
            assertEquals(1, fixture.peerHost.managementHandoffs(fixture.database).size)
            assertEquals(history, fixture.peerSession!!.history(FormKind.ENTRY, fixture.entry))
        }
    }
}
