package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.AndroidException
import dev.taypeer.bridge.DeviceSettingsView
import dev.taypeer.bridge.Host
import kotlinx.coroutines.*

/** A test seam for scheduling replies; all production validation and writes remain native. */
internal interface DeviceSettingsBackend {
    suspend fun load(): DeviceSettingsView
    suspend fun idle(seconds: UInt): DeviceSettingsView
    suspend fun clipboard(seconds: UInt?): DeviceSettingsView
    suspend fun name(value: String): DeviceSettingsView
}

private class NativeDeviceSettingsBackend(private val host: Deferred<Host>) : DeviceSettingsBackend {
    override suspend fun load() = host.await().deviceSettings()
    override suspend fun idle(seconds: UInt) = host.await().setIdleSeconds(seconds)
    override suspend fun clipboard(seconds: UInt?) = host.await().setClipboardSeconds(seconds)
    override suspend fun name(value: String) = host.await().setDeviceName(value)
}

internal enum class DeviceNameFailure { REQUIRED, TOO_LONG, PROFILE_IO, PROFILE_INVALID, PROFILE_BUSY, UNCERTAIN, OTHER }

/** Applied native values and an independent raw name draft, owned for the application lifetime. */
internal class DeviceSettingsState(private val backend: DeviceSettingsBackend, private val scope: CoroutineScope) {
    constructor(host: Deferred<Host>, scope: CoroutineScope) : this(NativeDeviceSettingsBackend(host), scope)

    var settings by mutableStateOf<DeviceSettingsView?>(null); private set
    var busy by mutableStateOf(false); private set
    var failed by mutableStateOf(false); private set
    var nameDraft by mutableStateOf<String?>(null); private set
    var nameFailure by mutableStateOf<DeviceNameFailure?>(null); private set
    private var nameRevision = 0L
    private var lastSubmittedNameRevision: Long? = null
    private var unresolvedNamePublication = false
    private val pendingNames = mutableSetOf<Long>()
    private val queue = ArrayDeque<Request>()
    private var active: Request? = null

    private sealed interface Request {
        val revision: Long
        data class Load(override val revision: Long) : Request
        data class Idle(val seconds: UInt, override val revision: Long) : Request
        data class Clipboard(val seconds: UInt?, override val revision: Long) : Request
        data class Name(val value: String, override val revision: Long) : Request
    }

    fun load() {
        if (active is Request.Load || queue.any { it is Request.Load }) return
        enqueue(Request.Load(nameRevision))
    }
    fun applyIdle(seconds: UInt) = enqueue(Request.Idle(seconds, nameRevision))
    fun applyClipboard(seconds: UInt?) = enqueue(Request.Clipboard(seconds, nameRevision))

    fun editName(value: String) {
        if (nameDraft == value) return
        nameRevision++; nameDraft = value; nameFailure = null
    }

    /** Enter is also an explicit retry after failure. An in-flight revision is never duplicated. */
    fun submitName() {
        val value = nameDraft ?: return
        if (nameRevision in pendingNames) return
        lastSubmittedNameRevision = nameRevision
        nameFailure = null
        if (!unresolvedNamePublication && pendingNames.isEmpty() && settings?.deviceName == value) return
        pendingNames += nameRevision
        enqueue(Request.Name(value, nameRevision))
    }

    /** A Done/Enter followed by blur submits the captured revision exactly once. */
    fun finishNameInput() {
        if (lastSubmittedNameRevision != nameRevision) submitName()
    }

    private fun enqueue(request: Request) {
        queue.addLast(request)
        if (busy) return
        busy = true
        scope.launch {
            try {
                while (queue.isNotEmpty()) {
                    val next = queue.removeFirst()
                    active = next
                    if (next !is Request.Name) failed = false
                    try {
                        val applied = withContext(Dispatchers.IO) {
                            when (next) {
                                is Request.Load -> backend.load()
                                is Request.Idle -> backend.idle(next.seconds)
                                is Request.Clipboard -> backend.clipboard(next.seconds)
                                is Request.Name -> backend.name(next.value)
                            }
                        }
                        acknowledge(next, applied)
                    } catch (cancelled: CancellationException) { throw cancelled }
                    catch (error: Exception) {
                        if (next is Request.Name) {
                            if (error is AndroidException.CommitUncertain) unresolvedNamePublication = true
                            if (nameRevision == next.revision) nameFailure = nameFailure(error)
                        } else failed = true
                    } finally {
                        if (next is Request.Name) pendingNames.remove(next.revision)
                        active = null
                    }
                }
            } finally {
                queue.clear(); pendingNames.clear(); active = null; busy = false
            }
        }
    }

    private fun acknowledge(request: Request, value: DeviceSettingsView) {
        val previous = settings
        settings = value
        if (request is Request.Name) unresolvedNamePublication = false
        if (nameRevision != request.revision) return
        if (request is Request.Name || nameDraft == null || nameDraft == previous?.deviceName) {
            nameDraft = value.deviceName
        }
        if (!unresolvedNamePublication && nameDraft == value.deviceName) nameFailure = null
    }

    private fun nameFailure(error: Exception) = when (error) {
        is AndroidException.DeviceNameRequired -> DeviceNameFailure.REQUIRED
        is AndroidException.DeviceNameTooLong -> DeviceNameFailure.TOO_LONG
        is AndroidException.ProfileIo -> DeviceNameFailure.PROFILE_IO
        is AndroidException.ProfileInvalid -> DeviceNameFailure.PROFILE_INVALID
        is AndroidException.ProfileBusy -> DeviceNameFailure.PROFILE_BUSY
        is AndroidException.CommitUncertain -> DeviceNameFailure.UNCERTAIN
        else -> DeviceNameFailure.OTHER
    }
}
