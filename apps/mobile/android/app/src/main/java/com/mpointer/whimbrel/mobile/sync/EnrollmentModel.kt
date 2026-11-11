package com.mpointer.whimbrel.mobile.sync

import androidx.annotation.StringRes
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.auth.GoogleSignInError
import com.mpointer.whimbrel.mobile.core.AccountPlan
import com.mpointer.whimbrel.mobile.core.CoreNative
import com.mpointer.whimbrel.mobile.core.CoreUserMessage
import com.mpointer.whimbrel.mobile.core.GoogleAuthResult
import com.mpointer.whimbrel.mobile.core.GroupKeyDecision
import com.mpointer.whimbrel.mobile.core.LimitHolder
import com.mpointer.whimbrel.mobile.core.SyncApiError
import com.mpointer.whimbrel.mobile.core.coreUserMessage
import com.mpointer.whimbrel.mobile.secrets.SyncSecrets
import com.mpointer.whimbrel.mobile.secrets.TursoSession
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** Where the phone is in device enrollment. No case carries a token or key, so logging a status is always safe. */
sealed interface EnrollmentStatus {
    /** The build has no usable Web client id / Worker URL. */
    data class NotConfigured(val message: String) : EnrollmentStatus
    data object SignedOut : EnrollmentStatus
    data object SigningIn : EnrollmentStatus

    /** Signed in, waiting for an approved desktop; [code] is this device's comparison code, shown on the Mac too. */
    data class Pending(val code: String) : EnrollmentStatus

    /** Approved, but no group key arrived (the handoff expired): re-approve this phone from the Mac. */
    data class WaitingForGroupKey(val code: String) : EnrollmentStatus

    /** No approved desktop on the account. The phone never creates the group key, so set up the Mac first. */
    data object NeedsDesktopFirst : EnrollmentStatus

    /** `409 device_limit`: the free plan's phone slot is taken. Nothing was enrolled. */
    data class DeviceLimit(val kind: String, val holders: List<LimitHolder>) : EnrollmentStatus

    /** Approved and the group key is stored. */
    data object Approved : EnrollmentStatus

    /**
     * The core's message for the failure, rendered with [CoreUserMessage.localized]; null when there is
     * none (a malformed reply, a local failure): show a generic error. Code `signin_expired` is "Sign in again".
     */
    data class Error(val message: CoreUserMessage?) : EnrollmentStatus

    /** The account revoked this phone: its keys, Google sign-in and local store are gone. */
    data object Removed : EnrollmentStatus
}

/** The device-limit screen's sentence: [resId] takes the holder's name, or the default when [holderName] is null. */
data class LimitMessage(@StringRes val resId: Int, val holderName: String?) {
    fun localized(context: (Int, Array<Any>) -> String): String =
        context(resId, arrayOf(holderName ?: context(R.string.limit_default_phone_name, emptyArray())))
}

/** A failed swap or waitlist request: the core's message when there is one, else the generic [fallback]. */
data class ActionError(val core: CoreUserMessage?, @StringRes val fallback: Int = R.string.action_error_generic) {
    fun localized(context: (Int, Array<Any>) -> String): String = core?.localized(context) ?: context(fallback, emptyArray())
}

data class SyncCredentials(val tursoToken: String, val tursoDatabaseUrl: String, val groupKeyBase64: String, val deviceId: String) {
    override fun toString() = "SyncCredentials(redacted)"
}

interface SyncCredentialSource {
    fun syncCredentials(): SyncCredentials?
    suspend fun renewIfDue(afterFailure: Boolean, force: Boolean)
}

/**
 * The phone's enrollment flow, ported from `EnrollmentModel.swift`: sign in, register, wait for a Mac's
 * approval, unwrap the group key, renew. The phone never generates the group key; it only receives it from
 * an approved desktop. The Turso token lives only in memory and is renewed by re-authenticating.
 *
 * Not thread-safe: call it from the main thread, with [scope] on the main dispatcher (the poll runs there).
 */
class EnrollmentModel(
    private val backend: EnrollmentBackend?,
    private val secrets: SyncSecrets,
    private val native: CoreNative,
    private val isPermanentError: (Throwable) -> Boolean,
    private val scope: CoroutineScope,
    private val deviceName: () -> String,
    private val clock: () -> Long,
    private val sleep: suspend (Long) -> Unit,
    configError: String? = null,
) : SyncCredentialSource {
    private val _status = MutableStateFlow(
        when {
            backend == null -> EnrollmentStatus.NotConfigured(configError.orEmpty())
            // Approved before: stay approved offline; resume() fetches a fresh token.
            secrets.groupKey != null && secrets.deviceSecretKey != null -> EnrollmentStatus.Approved
            else -> EnrollmentStatus.SignedOut
        },
    )
    val status: StateFlow<EnrollmentStatus> = _status.asStateFlow()

    private val _swapRequested = MutableStateFlow(false)

    /** This phone asked to replace the synced phone. */
    val swapRequested: StateFlow<Boolean> = _swapRequested.asStateFlow()

    private val _actionError = MutableStateFlow<ActionError?>(null)

    /** A failed swap or waitlist request, for the limit screen. */
    val actionError: StateFlow<ActionError?> = _actionError.asStateFlow()

    private val _deviceLabels = MutableStateFlow(DeviceLabels())

    /** Names of the account's devices and this phone's own id, for labelling where a meeting was recorded. Best effort: empty until the first refresh. */
    val deviceLabels: StateFlow<DeviceLabels> = _deviceLabels.asStateFlow()
    private var lastDeviceRefreshAtMs: Long? = null

    /** Called once per wipe, after the secrets are cleared: clears synced prefs and keys, the store and the sync model. */
    var onWipe: (() -> Unit)? = null

    /** The running waiting poll, if any. Exposed so tests can join it. */
    var pollJob: Job? = null
        private set
    private var resumeJob: Deferred<Unit>? = null

    /** The Worker's Turso token for the account's data database, restored from [SyncSecrets.tursoSession]. */
    private var turso: Pair<String, String>? = null
    private var lastAuthAtMs: Long? = null

    init {
        if (_status.value == EnrollmentStatus.Approved) {
            secrets.tursoSession?.let {
                turso = it.token to it.databaseUrl
                lastAuthAtMs = it.issuedAtMs
            }
        }
    }

    /** Sets the token in memory and on disk (null deletes both). */
    private fun saveTurso(value: Pair<String, String>?, issuedAtMs: Long? = null) {
        turso = value
        runCatching {
            secrets.tursoSession = value?.let { (token, url) -> TursoSession(token, url, issuedAtMs ?: clock()) }
        }
    }
    private var lastRenewAttemptAtMs: Long? = null

    /** This phone's server device id when a self-revoke failed (offline): retried at the next sign-in. */
    private var owedSelfRevoke: String? = null

    companion object {
        /** `listDevices` is a registry write: at most one per 5 minutes. */
        const val deviceRefreshIntervalMs = 300_000L

        /** The waitlist's checkboxes, in order: label and the Worker's exact `wants` string (same as the Mac). */
        val waitlistOptions: List<Pair<Int, String>> = listOf(
            R.string.waitlist_more_devices to "more_devices",
            R.string.waitlist_routines_while_closed to "routines_while_closed",
            R.string.waitlist_phone_notifications to "phone_notifications",
            R.string.waitlist_backups to "backups",
        )

        fun limitMessage(holders: List<LimitHolder>) = LimitMessage(R.string.limit_message, holders.firstOrNull()?.name)

        /** Same cadence as the Mac's waiting poll. */
        const val basePollSeconds = 3L
        const val maxPollSeconds = 30L

        /** Doubles after a failed poll, capped; back to the base after a good one. */
        fun nextPollInterval(current: Long, succeeded: Boolean): Long =
            if (succeeded) basePollSeconds else minOf(current * 2, maxPollSeconds)

        /** The Worker's permanent "device revoked" answer (403), and only that one. */
        fun isRevoked(e: Throwable) = e is SyncApiError.Http && e.status == 403 && "device revoked" in e.body

        /** The Worker refused to enrol a phone on an account with no desktop: nothing was enrolled. */
        private fun isDesktopRequired(e: Throwable) = e is SyncApiError.Http && e.status == 409 && "desktop_required" in e.body
    }

    /** Retrying won't help: the Worker refused this device/account (the core decides), or Google sign-in is gone. */
    fun isPermanent(e: Throwable): Boolean = e is GoogleSignInError.NotSignedIn || isPermanentError(e)

    /** The "Sign in with Google" button. */
    suspend fun signIn() {
        val backend = backend ?: return
        _status.value = EnrollmentStatus.SigningIn
        var requestKey: String? = null
        try {
            backend.signIn()
            val idToken = backend.idToken()
            val owed = owedSelfRevoke
            val oldKey = currentPublicKey()
            if (owed != null && oldKey != null) {
                selfRevoke(owed, oldKey)
                if (owedSelfRevoke != null) {
                    _status.value = EnrollmentStatus.NeedsDesktopFirst
                    return
                }
            }
            val publicKey = ensureDeviceKeypair()
            requestKey = publicKey
            handle(backend.authenticate(idToken, publicKey), publicKey)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            when {
                e is GoogleSignInError.Cancelled -> _status.value = EnrollmentStatus.SignedOut
                isRevoked(e) && requestKey != null -> wipeRevoked(requestKey)
                // The request's keypair was wiped meanwhile: land nothing.
                requestKey != null && currentPublicKey() != requestKey -> return
                isDesktopRequired(e) -> _status.value = EnrollmentStatus.NeedsDesktopFirst
                else -> _status.value = errorFor(e)
            }
        }
    }

    /**
     * On launch: an approved phone with a saved token renews only when due (no Google call, so no chooser,
     * on every restart); anything else re-checks now.
     */
    suspend fun launch() {
        if (_status.value == EnrollmentStatus.Approved && turso != null) renewIfDue(afterFailure = false, force = false) else resume()
    }

    /** For each token renewal: if this phone signed in before, re-check silently. Joins a re-check in flight. */
    suspend fun resume() {
        val job = resumeJob ?: scope.async { resumeOnce() }.also { job ->
            resumeJob = job
            job.invokeOnCompletion { if (resumeJob === job) resumeJob = null }
        }
        job.await()
    }

    private suspend fun resumeOnce() {
        val backend = backend ?: return
        val publicKey = currentPublicKey() ?: return
        lastRenewAttemptAtMs = clock()
        try {
            handle(backend.authenticate(backend.idToken(), publicKey), publicKey)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Offline or a passing failure: the user keeps what they see (approved stays approved).
            when {
                isRevoked(e) -> wipeRevoked(publicKey)
                // The keypair was wiped meanwhile: land nothing.
                currentPublicKey() != publicKey -> return
                // Android: silent renewal auto-selected a Google account with no desktop. Keep the keys
                // (the right account may come back) but stop syncing with this one's token.
                isDesktopRequired(e) -> {
                    saveTurso(null)
                    _status.value = EnrollmentStatus.NeedsDesktopFirst
                }
                // Refused outright: say so rather than show "approved" while every pull fails.
                _status.value == EnrollmentStatus.Approved && isPermanent(e) -> {
                    saveTurso(null)
                    _status.value = errorFor(e)
                }
            }
        }
    }

    override fun syncCredentials(): SyncCredentials? {
        if (_status.value != EnrollmentStatus.Approved) return null
        val (token, databaseUrl) = turso ?: return null
        val groupKey = secrets.groupKey ?: return null
        val deviceId = currentPublicKey() ?: return null
        return SyncCredentials(token, databaseUrl, groupKey, deviceId)
    }

    /** Re-authenticate before the Turso token expires, or after a failed pull. [force] (Sync now) skips the retry floor. */
    override suspend fun renewIfDue(afterFailure: Boolean, force: Boolean) {
        if (_status.value != EnrollmentStatus.Approved) return
        resumeJob?.let { return it.await() }
        val lastAttempt = if (force) null else lastRenewAttemptAtMs
        if (!SyncTiming.renewalDue(clock(), lastAuthAtMs, lastAttempt, afterFailure)) return
        resume()
    }

    /** No polling in the background. */
    fun enterBackground() = cancelPoll()

    /** Back in the foreground while waiting for approval: re-check now, which restarts the poll. */
    suspend fun enterForeground() {
        lastDeviceRefreshAtMs = null // the next sync refreshes the device list
        if (pollJob != null) return
        when (_status.value) {
            is EnrollmentStatus.Pending, is EnrollmentStatus.WaitingForGroupKey -> resume()
            else -> return
        }
    }

    /**
     * Best-effort cleanup of an honest device: forget keys, Google tokens and (via [onWipe]) the local store.
     * Status flips first, so a second revoked answer is a no-op. [forPublicKey] is the keypair the answer was
     * for: an answer for an older keypair must not wipe the current one.
     */
    fun wipeRevoked(forPublicKey: String) {
        // Fail toward wiping: skip only when a key was read and it is a different one.
        val current = runCatching { currentPublicKey() }.getOrNull()
        if (current != null && current != forPublicKey) return
        if (_status.value == EnrollmentStatus.Removed) return
        _status.value = EnrollmentStatus.Removed
        cancelPoll()
        saveTurso(null)
        owedSelfRevoke = null
        _swapRequested.value = false
        _actionError.value = null
        _deviceLabels.value = DeviceLabels()
        lastDeviceRefreshAtMs = null
        runCatching { secrets.deleteGroupKey() }
        runCatching { secrets.deleteDeviceSecretKey() }
        // The store first: the Google sign-out is best effort and must never stand between revoke and wipe.
        onWipe?.invoke()
        runCatching { backend?.signOut() }
    }

    private suspend fun handle(result: GoogleAuthResult, publicKey: String) {
        // The answer is for a keypair this phone no longer holds (wiped or replaced while in flight): land nothing.
        if (currentPublicKey() != publicKey) return
        val code = native.deviceComparisonCode(publicKey).orEmpty()
        when (result) {
            GoogleAuthResult.Pending -> {
                _status.value = EnrollmentStatus.Pending(code)
                nameDevice(publicKey)
                if (currentPublicKey() != publicKey) return
                startPolling(publicKey)
            }
            is GoogleAuthResult.DeviceLimit -> {
                cancelPoll()
                _swapRequested.value = false
                _status.value = EnrollmentStatus.DeviceLimit(result.kind, result.holders)
            }
            is GoogleAuthResult.Approved -> {
                _swapRequested.value = false
                val outcome = resolveGroupKey(result.wrappedGroupKey, publicKey)
                if (currentPublicKey() != publicKey) return
                when {
                    outcome is GroupKeyOutcome.NeedsDesktop -> {
                        // Approved but keyless, and no desktop to hand over a key: step out of the account.
                        // Revoke first: from the poll, cancelling it would cancel the revoke request.
                        selfRevoke(outcome.ownDeviceId, publicKey)
                        cancelPoll()
                        _status.value = EnrollmentStatus.NeedsDesktopFirst
                    }
                    secrets.groupKey != null -> {
                        _status.value = EnrollmentStatus.Approved
                        lastAuthAtMs = clock()
                        saveTurso(result.tursoToken to result.tursoDatabaseUrl, lastAuthAtMs)
                        lastRenewAttemptAtMs = lastAuthAtMs
                        // Name before cancelling: from the poll, the cancel would cancel this request.
                        nameDevice(publicKey)
                        cancelPoll()
                    }
                    else -> {
                        _status.value = EnrollmentStatus.WaitingForGroupKey(code)
                        startPolling(publicKey)
                    }
                }
            }
        }
    }

    private sealed interface GroupKeyOutcome {
        data object ResolvedOrWaiting : GroupKeyOutcome
        data class NeedsDesktop(val ownDeviceId: String?) : GroupKeyOutcome
    }

    /** The core's group-key decision table, except the phone never generates: where the Mac would, it needs a desktop. */
    private suspend fun resolveGroupKey(wrappedGroupKey: String?, publicKey: String): GroupKeyOutcome {
        val backend = backend ?: return GroupKeyOutcome.ResolvedOrWaiting
        val localKeyHeld = secrets.groupKey != null
        return when (GroupKeyDecision.decide(native, wrappedGroupKey != null, localKeyHeld, anotherApprovedDeviceExists = false)) {
            GroupKeyDecision.KEEP, GroupKeyDecision.WAIT_FOR_HANDOFF -> GroupKeyOutcome.ResolvedOrWaiting
            GroupKeyDecision.UNWRAP -> {
                val secretKey = secrets.deviceSecretKey ?: throw IllegalStateException("no device keypair")
                val devices = backend.listDevices(backend.idToken(), publicKey)
                val groupKey = devices.asSequence()
                    .filter { it.publicKey != publicKey && it.status == "approved" }
                    .firstNotNullOfOrNull { sender ->
                        jsonField(native.syncUnwrapGroupKey(secretKey, sender.publicKey, wrappedGroupKey.orEmpty()), "group_key_base64")
                    }
                if (groupKey != null && currentPublicKey() == publicKey) secrets.groupKey = groupKey
                GroupKeyOutcome.ResolvedOrWaiting
            }
            GroupKeyDecision.GENERATE -> {
                val devices = backend.listDevices(backend.idToken(), publicKey)
                val anotherApproved = devices.any { it.publicKey != publicKey && it.status == "approved" }
                if (GroupKeyDecision.decide(native, false, false, anotherApproved) == GroupKeyDecision.GENERATE) {
                    GroupKeyOutcome.NeedsDesktop(devices.firstOrNull { it.publicKey == publicKey }?.id)
                } else {
                    GroupKeyOutcome.ResolvedOrWaiting
                }
            }
        }
    }

    /** Revokes this phone's own row, then discards its device key; if the revoke fails the key stays and the next sign-in retries. */
    private suspend fun selfRevoke(deviceId: String?, publicKey: String) {
        // No id (own row not listed): nothing was revoked, so keep the key.
        val backend = backend ?: return
        if (deviceId == null) return
        try {
            backend.revokeDevice(backend.idToken(), publicKey, deviceId)
        } catch (_: Exception) {
            owedSelfRevoke = deviceId
            return
        }
        owedSelfRevoke = null
        runCatching { secrets.deleteDeviceSecretKey() }
    }

    /**
     * "Use this phone instead": becomes a pending swap for [holder], approved by a computer or by the old
     * phone. Unlike the Mac there is no fresh start: a phone can't make the group key.
     */
    suspend fun swap(replacing: LimitHolder) {
        val backend = backend ?: return
        val publicKey = currentPublicKey() ?: return
        _actionError.value = null
        try {
            backend.swapDevice(backend.idToken(), publicKey, replacing.id)
            if (currentPublicKey() != publicKey) return
            _swapRequested.value = true
            _status.value = EnrollmentStatus.Pending(native.deviceComparisonCode(publicKey).orEmpty())
            nameDevice(publicKey)
            if (currentPublicKey() != publicKey) return
            startPolling(publicKey)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            noteActionError(e, publicKey)
        }
    }

    /** True when the Worker took the request. */
    suspend fun joinWaitlist(wants: List<String>): Boolean {
        val backend = backend ?: return false
        val publicKey = currentPublicKey() ?: return false
        _actionError.value = null
        return try {
            backend.joinWaitlist(backend.idToken(), publicKey, wants)
            true
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            noteActionError(e, publicKey)
            false
        }
    }

    /** The account's plan, or null when this phone is not approved or the lookup fails (offline, token trouble). */
    suspend fun fetchAccountPlan(): AccountPlan? {
        if (_status.value != EnrollmentStatus.Approved) return null
        val backend = backend ?: return null
        val publicKey = currentPublicKey() ?: return null
        return try {
            backend.accountPlan(backend.idToken(), publicKey)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }
    }

    /** An error answer for a keypair this phone no longer holds (wiped meanwhile) lands nothing. */
    private fun noteActionError(e: Exception, publicKey: String) {
        if (currentPublicKey() != publicKey) return
        _actionError.value = ActionError(coreMessageFor(e))
    }

    /**
     * Fetches the account's device list for [deviceLabels]. Silent on any failure (offline, token trouble):
     * labels fall back to "Device xxxxxx". Throttled: runs when names are missing, after the app became
     * active ([enterForeground]), or 5 minutes after the last one.
     */
    suspend fun refreshDeviceLabels() {
        val last = lastDeviceRefreshAtMs
        if (_deviceLabels.value.names.isNotEmpty() && last != null && clock() - last < deviceRefreshIntervalMs) return
        val backend = backend ?: return
        if (_status.value != EnrollmentStatus.Approved) return
        val publicKey = currentPublicKey() ?: return
        val records = try {
            backend.listDevices(backend.idToken(), publicKey)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            return
        }
        // Wiped or replaced while the list was in flight: land nothing.
        if (currentPublicKey() != publicKey) return
        lastDeviceRefreshAtMs = clock()
        _deviceLabels.value = DeviceLabels(DeviceLabels.names(records), publicKey)
    }

    /** Names this phone's row (while pending, for the Mac's approve prompt; again once approved). Best effort. */
    private suspend fun nameDevice(publicKey: String) {
        val backend = backend ?: return
        try {
            backend.setDeviceName(backend.idToken(), publicKey, deviceName())
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
        }
    }

    /** Re-ask the Worker until approved, backing off on transient failures, stopping on permanent ones. */
    private suspend fun startPolling(publicKey: String) {
        // Not from a cancelled poll (e.g. `handle` after its own awaits, once backgrounded).
        val backend = backend ?: return
        if (pollJob != null || !currentCoroutineContext().isActive) return
        val job = scope.launch(start = CoroutineStart.LAZY) {
            var interval = basePollSeconds
            while (isActive) {
                try {
                    sleep(interval)
                } catch (_: CancellationException) {
                    break
                }
                try {
                    val result = backend.authenticate(backend.idToken(), publicKey)
                    // The request may ignore cancellation: if the app went to the background meanwhile, drop the answer.
                    if (!isActive) break
                    handle(result, publicKey)
                    if (_status.value == EnrollmentStatus.Approved) break
                    interval = nextPollInterval(interval, succeeded = true)
                } catch (e: Exception) {
                    // Cancelled meanwhile (backgrounded): the error is stale, don't act on it.
                    if (!isActive) break
                    if (isRevoked(e)) {
                        wipeRevoked(publicKey)
                        break
                    }
                    if (isPermanent(e)) {
                        _status.value = errorFor(e)
                        break
                    }
                    interval = nextPollInterval(interval, succeeded = false)
                }
            }
            // A cancelled poll was already cleared by whoever cancelled it, and a new one may have started since.
            if (isActive) pollJob = null
        }
        pollJob = job
        job.start()
    }

    private fun cancelPoll() {
        pollJob?.cancel()
        pollJob = null
    }

    private fun coreMessageFor(e: Throwable): CoreUserMessage? = when (e) {
        is GoogleSignInError.NotSignedIn -> CoreUserMessage(code = "signin_expired", message = "")
        is GoogleSignInError.Failed -> CoreUserMessage(code = "google_signin_failed", message = "")
        is SyncApiError -> coreUserMessage(native, e)
        else -> null
    }

    private fun errorFor(e: Throwable) = EnrollmentStatus.Error(coreMessageFor(e))

    private fun ensureDeviceKeypair(): String {
        currentPublicKey()?.let { return it }
        val generated = native.syncGenerateDeviceKeypair()
        val secretKey = jsonField(generated, "secret_key_base64") ?: throw IllegalStateException("keypair generation failed")
        val publicKey = jsonField(generated, "public_key_base64") ?: throw IllegalStateException("keypair generation failed")
        secrets.deviceSecretKey = secretKey
        return publicKey
    }

    private fun currentPublicKey(): String? =
        secrets.deviceSecretKey?.let { jsonField(native.syncDevicePublicKey(it), "public_key_base64") }

    private fun jsonField(json: String?, key: String): String? = json?.let {
        runCatching { Json.parseToJsonElement(it).jsonObject[key]?.jsonPrimitive?.contentOrNull }.getOrNull()
    }
}
