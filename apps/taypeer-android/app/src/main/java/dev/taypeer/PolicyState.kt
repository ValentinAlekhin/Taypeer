package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.CalibrationView
import dev.taypeer.bridge.PolicyDetailsView
import dev.taypeer.bridge.PolicyView

internal enum class PolicyPage { PROTECTION, LIMITS }
internal enum class PolicyFailure { READ, INPUT, REJECTED, UNCERTAIN, CALIBRATION }
internal data class PolicySource(val database: String, val workingCopyPath: String, val generation: ULong)
private data class PolicyAddress(val database: String, val workingCopyPath: String)

/** Exact submitted intent; its password is never printed or persisted and is dropped on lock. */
internal class PolicyRequest(val source: PolicySource, val page: PolicyPage, val value: PolicyView,
    val operation: String, password: String?) {
    private var password = password
    internal fun execute(session: dev.taypeer.bridge.DocumentSession) = session.setPolicy(value, password, operation)
    internal fun clearPassword() { password = null }
}

/** UI request ownership only; existing Rust commands own validation, rotation and receipts. */
internal class PolicyState(private val commands: SessionCommands, private val source: () -> PolicySource?,
    private val canManage: () -> Boolean, private val confirmed: () -> Unit) {
    var page by mutableStateOf<PolicyPage?>(null); private set
    var details by mutableStateOf<PolicyDetailsView?>(null); private set
    var preview by mutableStateOf<CalibrationView?>(null); private set
    var attachment by mutableStateOf(""); private set
    var total by mutableStateOf(""); private set
    var target by mutableStateOf<UInt?>(null); private set
    var password by mutableStateOf(""); private set
    var busy by mutableStateOf(false); private set
    var calibrating by mutableStateOf(false); private set
    var failure by mutableStateOf<PolicyFailure?>(null); private set
    private var requests by mutableStateOf<Map<PolicyAddress, PolicyRequest>>(emptyMap())
    private var epoch = 0L
    private var targetRevision = 0L
    val managing get() = canManage()
    val retained get() = source()?.let { requests[address(it)] }
    val editable get() = details != null && managing && !busy && retained == null
    val closeEnabled get() = !busy && retained == null
    private fun address(value: PolicySource) = PolicyAddress(value.database, value.workingCopyPath)
    private fun current(value: PolicySource, expectedEpoch: Long) = page != null && epoch == expectedEpoch && source() == value

    /** Called only after DocumentState's existing Save/Discard/Stay navigation guard. */
    fun show(value: PolicyPage) {
        val selected = source() ?: return
        if (busy) return
        epoch++; targetRevision++
        val held = requests[address(selected)]
        page = held?.page ?: value
        details = null; preview = null; attachment = ""; total = ""; target = null; password = ""
        calibrating = false; failure = null
        if (held != null) {
            attachment = PolicyUnits.editableMib(held.value.attachmentBytes)
            total = PolicyUnits.editableMib(held.value.totalAttachmentBytes)
            target = held.value.kdfTargetMs
            failure = PolicyFailure.UNCERTAIN
            return // Exact receipt retry precedes inspection of a possibly changed projection.
        }
        reload()
    }

    fun reload() { if (retained != null) retry() else read(preserveInput = false) }
    private fun read(preserveInput: Boolean) {
        val selected = source() ?: return
        if (page == null || busy) return
        val expectedEpoch = epoch
        busy = true; failure = null
        val accepted = commands.execute(navigate = false, command = { it.policyDetails() }, started = {},
            completed = { view -> if (current(selected, expectedEpoch)) {
                details = view
                if (!preserveInput) {
                    val held = requests[address(selected)]?.value ?: view.policy
                    attachment = PolicyUnits.editableMib(held.attachmentBytes)
                    total = PolicyUnits.editableMib(held.totalAttachmentBytes)
                    target = held.kdfTargetMs
                }
                busy = false
                failure = if (retained != null) PolicyFailure.UNCERTAIN else null
            } }, failed = { if (current(selected, expectedEpoch)) { busy = false; failure = PolicyFailure.READ } })
        if (!accepted && current(selected, expectedEpoch)) { busy = false; failure = PolicyFailure.READ }
    }

    fun editAttachment(value: String) { if (editable) { attachment = value; failure = null } }
    fun editTotal(value: String) { if (editable) { total = value; failure = null } }
    fun editPassword(value: String) { if (editable) { password = value; failure = null } }
    fun selectTarget(value: UInt) {
        if (!editable || target == value) return
        target = value; targetRevision++; preview = null; failure = null
    }

    /** Explicit Native work only; changing a target never starts calibration. */
    fun calibrate() {
        if (!editable || calibrating) return
        val selected = source() ?: return
        val milliseconds = target ?: return
        val expectedEpoch = epoch
        val revision = targetRevision
        calibrating = true; preview = null; failure = null
        val accepted = commands.execute(navigate = false,
            command = { it.calibrateProtection(milliseconds) }, started = {}, completed = { value ->
                if (current(selected, expectedEpoch)) {
                    calibrating = false
                    if (revision == targetRevision && target == milliseconds) preview = value
                }
            }, failed = { if (current(selected, expectedEpoch)) {
                calibrating = false
                if (revision == targetRevision) failure = PolicyFailure.CALIBRATION
            } })
        if (!accepted && current(selected, expectedEpoch)) { calibrating = false; failure = PolicyFailure.CALIBRATION }
    }

    fun apply() {
        if (retained != null) { retry(); return }
        if (!editable || calibrating) return
        val selected = source() ?: return
        val activePage = page ?: return
        val applied = details?.policy ?: return
        val milliseconds = target ?: return
        val one = if (activePage == PolicyPage.LIMITS) PolicyUnits.parseMib(attachment) else applied.attachmentBytes
        val all = if (activePage == PolicyPage.LIMITS) PolicyUnits.parseMib(total) else applied.totalAttachmentBytes
        if (one == null || all == null) { failure = PolicyFailure.INPUT; return }
        val value = PolicyView(one, all, if (activePage == PolicyPage.PROTECTION) milliseconds else applied.kdfTargetMs)
        val key = address(selected)
        if (requests.size >= 256 && key !in requests) { failure = PolicyFailure.UNCERTAIN; return }
        requests = requests + (key to PolicyRequest(selected, activePage, value, newOperation(), password.takeIf { value.kdfTargetMs != applied.kdfTargetMs }))
        retry()
    }

    /** Native receipt is attempted before any new inspection or intent reconstruction. */
    fun retry() {
        val selected = source() ?: return
        val request = requests[address(selected)] ?: return
        if (page == null || busy) return
        val expectedEpoch = epoch
        var acknowledged = false
        busy = true; failure = null
        val accepted = commands.execute(navigate = false, command = request::execute, started = {},
            completed = { if (current(selected, expectedEpoch)) {
                acknowledged = true
                if (requests[address(selected)] === request) requests = requests - address(selected)
                request.clearPassword(); busy = false
                clearRoute()
                confirmed()
            } }, failed = { if (current(selected, expectedEpoch)) {
                busy = false
                if (!acknowledged) failure = PolicyFailure.UNCERTAIN
            } }, rejected = { if (current(selected, expectedEpoch) && !acknowledged) {
                if (requests[address(selected)] === request) requests = requests - address(selected)
                request.clearPassword(); busy = false; failure = PolicyFailure.REJECTED
            } })
        if (!accepted && current(selected, expectedEpoch)) { busy = false; failure = PolicyFailure.UNCERTAIN }
    }

    fun close() { if (closeEnabled) clearRoute() }
    internal fun refreshIfOpen() {
        targetRevision++; preview = null
        if (page != null && details != null && !busy && retained == null) read(preserveInput = true)
    }
    private fun clearRoute() {
        epoch++; targetRevision++
        page = null; details = null; preview = null; password = ""; attachment = ""; total = ""; target = null
        busy = false; calibrating = false; failure = null
    }
    /** Immediate lock/switch cleanup; only exact public intent remains for reconciliation. */
    internal fun reset() { requests.values.forEach(PolicyRequest::clearPassword); clearRoute() }
}
