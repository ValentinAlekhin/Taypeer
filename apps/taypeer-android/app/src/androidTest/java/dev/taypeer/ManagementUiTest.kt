package dev.taypeer

import android.content.res.Configuration
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.concurrent.TimeUnit
import java.util.Locale

/** Product handlers backed by two real hosts, isolated Binder workers and authenticated Iroh. */
class ManagementUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun selectingDoesNotClaimReadinessAndBothExplicitUiActionsApplyOneTransfer() {
        PublicManagementFixture().use { fixture ->
            fixture.prepare()
            Actor(fixture, false).use { source -> Actor(fixture, true).use { peer ->
                var displayed by mutableStateOf(source)
                compose.setContent { Content(displayed, fixture.name) }
                val sourceBefore = File(fixture.sourceCopy.path).readBytes()
                val peerBefore = File(fixture.peerCopy.path).readBytes()
                val history = fixture.sourceSession.history(FormKind.ENTRY, fixture.entry)
                compose.runOnIdle { source.state.openTransfer() }
                await { source.state.route is ManagementRoute.Transfer }
                compose.onNodeWithTag("confirm-management-transfer").assertIsNotEnabled()
                compose.onNodeWithTag("management-recipient-${fixture.recipient}").performClick().assertIsSelected()
                compose.onNodeWithTag("management-peer-observation").assertDoesNotExist()
                assertTrue(fixture.sourceHost.managementHandoffs(fixture.database).isEmpty())
                assertArrayEquals(sourceBefore, File(fixture.sourceCopy.path).readBytes())
                assertArrayEquals(peerBefore, File(fixture.peerCopy.path).readBytes())
                var operation = ""
                compose.runOnIdle { operation = (source.state.route as ManagementRoute.Transfer).operation }
                assertTrue(operation.matches(Regex("[0-9a-f]{64}")))
                proposeThroughUi(fixture, source, operation)
                val intent = source.state.row(operation)!!.intent
                compose.onNodeWithTag("management-recipient-${fixture.recipient}").assertIsNotEnabled().assertIsSelected()
                compose.onNodeWithTag("management-peer-observation").assertDoesNotExist()
                compose.onNodeWithTag("check-management-peer").performScrollTo().performClick()
                await { source.state.observation != null && !source.state.observing }
                compose.runOnIdle {
                    val ready = source.state.observation!!.readiness as ManagementReadinessView.Ready
                    assertEquals(intent, source.state.observation!!.intent)
                    assertEquals(fixture.recipient, ready.device)
                    assertEquals(fixture.peerSession!!.generation(), ready.generation)
                }
                compose.runOnIdle { displayed = peer; peer.state.load() }
                await { peer.state.row(operation)?.phase == ManagementPhase.WAITING_DECISION }
                compose.onNodeWithTag("review-management-$operation").performScrollTo().performClick()
                compose.onNodeWithTag("cancel-form").performClick()
                await { peer.state.route == null }
                assertEquals(ManagementPhase.WAITING_DECISION, fixture.peerHost.managementHandoffs(fixture.database).single().phase)
                compose.onNodeWithTag("review-management-$operation").performScrollTo().performClick()
                compose.onNodeWithTag("confirm-management-accept").performClick()
                await { peer.state.route == null && peer.state.row(operation)?.phase == ManagementPhase.CONSENT_RECORDED }
                assertFalse(fixture.peerSession!!.overview("").managing)
                assertTrue(fixture.sourceSession.overview("").managing)
                compose.onNodeWithTag("review-management-$operation").assertDoesNotExist()
                compose.runOnIdle { displayed = source }
                compose.onNodeWithTag("confirm-management-transfer").performClick()
                await { !source.state.busy(operation) }
                // The actual UI retry initiates the source commit; delivery/apply remains actual Runtime work.
                PublicManagementFixture.await("PUBLIC UI transfers source authority") { !fixture.sourceSession.overview("").managing }
                val completed = fixture.complete(intent)
                compose.runOnIdle { source.state.load() }
                await { source.state.row(operation)?.phase == ManagementPhase.COMPLETED }
                compose.onNodeWithTag("confirm-management-transfer").assertIsNotEnabled()
                assertEquals(intent, completed.intent)
                assertNotNull(completed.applied)
                assertTrue(fixture.peerSession!!.overview("").managing)
                assertEquals(1, fixture.sourceHost.managementHandoffs(fixture.database).size)
                assertEquals(1, fixture.peerHost.managementHandoffs(fixture.database).size)
                assertEquals(history, fixture.sourceSession.history(FormKind.ENTRY, fixture.entry))
                assertEquals(history, fixture.peerSession!!.history(FormKind.ENTRY, fixture.entry))
                assertEquals(fixture.savedPassword, fixture.peerSession!!.reveal(fixture.entry, null, null))
            } }
        }
    }

    @Test fun explicitRejectUiPersistsRefusalWithoutChangingEitherEncryptedArchive() {
        PublicManagementFixture().use { fixture ->
            fixture.prepare()
            val proposal = fixture.propose()
            val sourceBefore = File(fixture.sourceCopy.path).readBytes()
            val peerBefore = File(fixture.peerCopy.path).readBytes()
            Actor(fixture, true).use { peer ->
                compose.setContent { Content(peer, fixture.name) }
                await { peer.state.row(proposal.intent.operation) != null }
                compose.onNodeWithTag("review-management-${proposal.intent.operation}").performScrollTo().performClick()
                compose.onNodeWithTag("reject-management").performScrollTo().performClick()
                await { peer.state.route == null && peer.state.row(proposal.intent.operation)?.phase == ManagementPhase.REJECTED }
                compose.onNodeWithTag("retry-management-${proposal.intent.operation}").assertDoesNotExist()
                assertEquals(ManagementPhase.REJECTED, fixture.sourceHost.retryManagement(proposal.intent).phase)
                assertArrayEquals(sourceBefore, File(fixture.sourceCopy.path).readBytes())
                assertArrayEquals(peerBefore, File(fixture.peerCopy.path).readBytes())
                assertTrue(fixture.sourceSession.overview("").managing)
                assertFalse(fixture.peerSession!!.overview("").managing)
            }
        }
    }

    @Test fun interruptedAcceptKeepsOnlyExactResumeAcrossLockAndRealHostRestart() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        PublicManagementFixture().use { fixture ->
            fixture.prepare()
            val proposal = fixture.propose()
            val before = File(fixture.sourceCopy.path).readBytes()
            Actor(fixture, true).use { peer ->
                var displayed by mutableStateOf(peer)
                compose.setContent { Content(displayed, fixture.name) }
                await { peer.state.row(proposal.intent.operation) != null }
                compose.onNodeWithTag("review-management-${proposal.intent.operation}").performScrollTo().performClick()
                fixture.peerHost.stopExchange()
                compose.onNodeWithTag("confirm-management-accept").performClick()
                await { !peer.state.busy(proposal.intent.operation) && peer.state.problem(proposal.intent.operation) == ManagementProblem.UNCONFIRMED }
                compose.runOnIdle { peer.state.load() }
                await { peer.state.row(proposal.intent.operation)?.phase == ManagementPhase.ACCEPT_REQUESTED }
                compose.onNodeWithTag("reject-management").assertDoesNotExist()
                val configuration = Configuration(context.resources.configuration).apply { setLocale(Locale.ENGLISH) }
                val continueLabel = context.createConfigurationContext(configuration).getString(R.string.continue_action)
                compose.onNodeWithTag("confirm-management-accept").assertTextContains(continueLabel)
                compose.onNodeWithTag("cancel-form").performClick()
                await { peer.state.route == null }
                compose.runOnIdle { peer.state.lock() }
                peer.close()
                fixture.restartPeer()
                Actor(fixture, true).use { reopened ->
                    compose.runOnIdle { displayed = reopened }
                    await { reopened.state.row(proposal.intent.operation)?.phase == ManagementPhase.ACCEPT_REQUESTED }
                    compose.onNodeWithTag("review-management-${proposal.intent.operation}").assertDoesNotExist()
                    compose.onNodeWithTag("retry-management-${proposal.intent.operation}").performScrollTo().performClick()
                    await { !reopened.state.busy(proposal.intent.operation) && reopened.state.row(proposal.intent.operation)?.phase == ManagementPhase.CONSENT_RECORDED }
                    assertEquals(proposal.intent, reopened.state.row(proposal.intent.operation)!!.intent)
                    assertArrayEquals(before, File(fixture.sourceCopy.path).readBytes())
                    fixture.complete(proposal.intent)
                    assertEquals(1, fixture.sourceHost.managementHandoffs(fixture.database).size)
                    assertEquals(1, fixture.peerHost.managementHandoffs(fixture.database).size)
                }
            }
        }
    }

    private fun proposeThroughUi(fixture: PublicManagementFixture, actor: Actor, operation: String) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        while (true) {
            val sourceRevision = fixture.sourceSession.applicationState()?.revision
            val peerRevision = fixture.peerSession!!.applicationState()?.revision
            compose.onNodeWithTag("confirm-management-transfer").performClick()
            await { !actor.state.busy(operation) && (actor.state.row(operation) != null || actor.state.problem(operation) != null) }
            if (fixture.peerHost.managementHandoffs(fixture.database).any { it.intent.operation == operation }) return
            compose.runOnIdle { assertEquals(ManagementProblem.PENDING_APPLY, actor.state.problem(operation)) }
            PublicManagementFixture.await("PUBLIC UI observes a newer actual automatic application", deadline) {
                fixture.sourceSession.applicationState()?.let { it.revision != sourceRevision && it.status != ApplicationPhase.FAILED } == true ||
                    fixture.peerSession!!.applicationState()?.let { it.revision != peerRevision && it.status != ApplicationPhase.FAILED } == true
            }
            assertTrue(System.nanoTime() < deadline)
            compose.runOnIdle { assertEquals(operation, (actor.state.route as ManagementRoute.Transfer).operation) }
        }
    }

    @Composable private fun Content(actor: Actor, name: String) {
        TaypeerTheme(ThemeMode.SYSTEM, UiLanguage.ENGLISH) {
            if (actor.state.route != null) ManagementScreen(actor.state, name, actor.name, actor.managing)
            else Column(Modifier.verticalScroll(rememberScrollState())) { ManagementHandoffs(actor.state, actor.state::openDecision) }
        }
    }
    private fun await(condition: () -> Boolean) = compose.waitUntil(30_000, condition)

    private inner class Actor(private val fixture: PublicManagementFixture, private val peer: Boolean) : AutoCloseable {
        private val job = SupervisorJob()
        private val scope = CoroutineScope(job + Dispatchers.Main.immediate)
        private fun session() = if (peer) fixture.peerSession!! else fixture.sourceSession
        private val binding = session().let { ManagementUiSession(it, fixture.database, it.generation(), 1) }
        val name = (if (peer) fixture.peerHost else fixture.sourceHost).deviceSettings().deviceName
        var managing by mutableStateOf(session().overview("").managing); private set
        val state = ManagementState(scope, { if (peer) fixture.peerHost else fixture.sourceHost },
            session = { binding }, current = { it === binding && (if (peer) fixture.peerSession else fixture.sourceSession) === it.session },
            changed = { expected -> scope.launch {
                val value = withContext(Dispatchers.IO) { expected.session.overview("").managing }
                if (expected === binding) managing = value
            } })
        init { compose.runOnIdle { state.selectDatabase(fixture.database) } }
        override fun close() { job.cancel(); runBlocking { job.join() } }
    }
}
