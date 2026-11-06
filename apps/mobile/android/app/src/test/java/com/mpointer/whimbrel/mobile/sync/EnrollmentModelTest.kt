package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.auth.GoogleSignInError
import com.mpointer.whimbrel.mobile.core.CoreUserMessage
import com.mpointer.whimbrel.mobile.core.FakeSyncNative
import com.mpointer.whimbrel.mobile.core.GoogleAuthResult
import com.mpointer.whimbrel.mobile.core.LimitHolder
import com.mpointer.whimbrel.mobile.core.SyncApiError
import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord
import com.mpointer.whimbrel.mobile.core.SyncWorkerClient
import com.mpointer.whimbrel.mobile.secrets.FakeCipher
import com.mpointer.whimbrel.mobile.secrets.SecretVault
import com.mpointer.whimbrel.mobile.secrets.SyncSecrets
import java.nio.file.Files
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Deterministic stand-in for the core's crypto: keypair n is `sk-n`/`pk-n`, a wrapped key is
 * `wrap|sender|recipient|groupKey`, and the group-key decision and permanence follow the core's tables.
 */
internal class FakeCoreNative : FakeSyncNative() {
    private var generated = 0

    override fun syncGenerateDeviceKeypair(): String {
        generated += 1
        return """{"public_key_base64":"pk-gen$generated","secret_key_base64":"sk-gen$generated"}"""
    }

    override fun syncDevicePublicKey(secretKeyBase64: String) = publicOf(secretKeyBase64)?.let { """{"public_key_base64":"$it"}""" }
    override fun deviceComparisonCode(publicKeyBase64: String) = "code($publicKeyBase64)"

    override fun syncUnwrapGroupKey(recipientSecretKeyBase64: String, senderPublicKeyBase64: String, wrappedGroupKeyBase64: String): String? {
        val (tag, sender, recipient, groupKey) = wrappedGroupKeyBase64.split("|").takeIf { it.size == 4 } ?: return null
        val ok = tag == "wrap" && sender == senderPublicKeyBase64 && recipient == publicOf(recipientSecretKeyBase64)
        return if (ok) """{"group_key_base64":"$groupKey"}""" else null
    }

    // core/api group_key_decision, as the JSON string the FFI returns.
    override fun syncGroupKeyDecision(wrappedKeyPresent: Boolean, localKeyHeld: Boolean, anotherApprovedDeviceExists: Boolean) =
        when {
            wrappedKeyPresent -> "\"unwrap\""
            localKeyHeld -> "\"keep\""
            anotherApprovedDeviceExists -> "\"wait_for_handoff\""
            else -> "\"generate\""
        }

    // core/api is_permanent_failure: 401 and 403.
    override fun syncWorkerErrorIsPermanent(errorJson: String) =
        Regex("\"status\":(\\d+)").find(errorJson)?.groupValues?.get(1).let { it == "401" || it == "403" }

    override fun syncWorkerErrorCode(errorJson: String) = """{"code":"from_core","message":"core says no"}"""

    companion object {
        fun publicOf(secret: String) = if (secret.startsWith("sk-")) "pk-" + secret.removePrefix("sk-") else null
        fun wrap(sender: String, recipient: String, groupKey: String) = "wrap|$sender|$recipient|$groupKey"
    }
}

/** Scripted Google sign-in and sync Worker, as `EnrollmentModelTests.swift`'s FakeBackend. */
class FakeBackend : EnrollmentBackend {
    var signInError: Exception? = null

    /** Answers to successive `authenticate` calls; each gets the caller's public key. */
    val authReplies = ArrayDeque<(String) -> GoogleAuthResult>()
    var devices = listOf<SyncDeviceRecord>()
    var signInCalls = 0
    var authCalls = 0
    var idTokenError: Exception? = null

    /** Like a cancellable request: `idToken()` and `revokeDevice` throw when called from a cancelled coroutine. */
    var throwsWhenCancelled = false

    /** This `authenticate` call (1-based) blocks until its coroutine is cancelled and then still answers. */
    var blockingAuthCall: Int? = null
    var blockingAuthError: Exception? = null

    /** While set, `authenticate` waits for it before answering (a request in flight). */
    var authGate: CompletableDeferred<Unit>? = null
    var revokeError: Exception? = null
    var setNameError: Exception? = null
    var signOutCalls = 0
    var signOutError: Exception? = null
    val deviceNames = mutableListOf<String>()
    val revokedTargets = mutableListOf<String>()

    override suspend fun signIn() {
        signInCalls += 1
        signInError?.let { throw it }
    }

    override suspend fun idToken(): String {
        if (throwsWhenCancelled && !currentCoroutineContext().isActive) throw CancellationException("cancelled")
        idTokenError?.let { throw it }
        return "id-token"
    }

    override suspend fun authenticate(idToken: String, devicePublicKey: String): GoogleAuthResult {
        authCalls += 1
        if (authCalls == blockingAuthCall) {
            while (currentCoroutineContext().isActive) withContext(NonCancellable) { delay(5) }
            blockingAuthError?.let { throw it }
            return GoogleAuthResult.Pending
        }
        authGate?.await()
        return authReplies.removeFirstOrNull()?.invoke(devicePublicKey) ?: GoogleAuthResult.Pending
    }

    override suspend fun listDevices(idToken: String, devicePublicKey: String): List<SyncDeviceRecord> {
        listDevicesCalls += 1
        return devices
    }

    override suspend fun revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) {
        if (throwsWhenCancelled && !currentCoroutineContext().isActive) throw CancellationException("cancelled")
        revokeError?.let { throw it }
        revokedTargets += targetDeviceId
    }

    override suspend fun setDeviceName(idToken: String, devicePublicKey: String, name: String) {
        deviceNames += name
        setNameError?.let { throw it }
    }

    var swapError: Exception? = null
    var waitlistError: Exception? = null
    var listDevicesCalls = 0
    val swappedTargets = mutableListOf<String>()
    val waitlistWants = mutableListOf<List<String>>()

    /** While set, `swapDevice` and `joinWaitlist` wait for it before answering (a request in flight). */
    var actionGate: CompletableDeferred<Unit>? = null

    override suspend fun swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) {
        actionGate?.await()
        swapError?.let { throw it }
        swappedTargets += replaceDeviceId
    }

    override suspend fun joinWaitlist(idToken: String, devicePublicKey: String, wants: List<String>) {
        actionGate?.await()
        waitlistError?.let { throw it }
        waitlistWants += wants
    }

    override fun signOut() {
        signOutCalls += 1
        signOutError?.let { throw it }
    }
}

/** Records poll sleeps and stops the loop (by throwing, like a cancelled delay) after `limit` of them. */
class FakeSleeper {
    val intervals = mutableListOf<Long>()
    var limit = 10

    fun sleep(seconds: Long) {
        intervals += seconds
        if (intervals.size > limit) throw CancellationException("sleeper limit")
    }
}

private fun device(publicKey: String, status: String = "approved", id: String = "mac") =
    SyncDeviceRecord(id = id, accountId = "acct", publicKey = publicKey, name = "Mac", platform = "macos", status = status, enrolledAtUnixMs = 0)

/** A Worker HTTP failure carrying the core's error envelope, as `SyncWorkerClient` throws it. */
private fun http(status: Int, body: String) =
    SyncApiError.Http(status, body, """{"error":{"kind":"http","status":$status,"body":${JsonPrimitive(body)}}}""")

private val revoked = http(403, """{"error":"device revoked"}""")
private val offline = SyncApiError.Network("""{"error":{"kind":"network"}}""")

@OptIn(ExperimentalCoroutinesApi::class)
class EnrollmentModelTest {
    private val backend = FakeBackend()
    private val sleeper = FakeSleeper()
    private val native = FakeCoreNative()
    private val secrets = SyncSecrets(SecretVault(Files.createTempDirectory("vault").toFile(), FakeCipher()))
    private var now = 1_000_000_000L
    private var wipes = 0

    private fun TestScope.makeModel(
        backend: EnrollmentBackend? = this@EnrollmentModelTest.backend,
        sleep: suspend (Long) -> Unit = { sleeper.sleep(it) },
    ) = EnrollmentModel(
        backend = backend,
        secrets = secrets,
        native = native,
        isPermanentError = SyncWorkerClient(native, "")::isPermanent,
        scope = backgroundScope,
        deviceName = { "Pixel Test" },
        clock = { now },
        sleep = sleep,
        configError = if (backend == null) "Google sign-in is not configured" else null,
    )

    private fun TestScope.makeWipingModel() = makeModel().also { it.onWipe = { wipes += 1 } }

    private fun storedPublicKey() = FakeCoreNative.publicOf(secrets.deviceSecretKey!!)!!

    /** Saves a device keypair and a group key, as after an earlier approval. */
    private fun seedApprovedPhone(): String {
        secrets.deviceSecretKey = "sk-seed"
        secrets.groupKey = "group-seed"
        return "pk-seed"
    }

    private fun approved(wrapped: String? = null) = GoogleAuthResult.Approved("t", "u", wrapped)

    /** "Approved, no key, no other desktop": the server lists only this phone, approved. */
    private fun approvedNoKeyReply(): (String) -> GoogleAuthResult = { phoneKey ->
        backend.devices = listOf(device(phoneKey, id = "phone-id"))
        approved()
    }

    private fun assertWiped(model: EnrollmentModel) {
        assertEquals(EnrollmentStatus.Removed, model.status.value)
        assertNull(secrets.groupKey)
        assertNull(secrets.deviceSecretKey)
        assertEquals("Google tokens deleted", 1, backend.signOutCalls)
        assertEquals("local store wiped once", 1, wipes)
    }

    private fun assertError(model: EnrollmentModel) =
        assertTrue("expected error, got ${model.status.value}", model.status.value is EnrollmentStatus.Error)

    @Test
    fun `pollIntervalDoublesOnFailureCapsAndResetsOnSuccess`() {
        var interval = EnrollmentModel.basePollSeconds
        val seen = (0 until 5).map { interval = EnrollmentModel.nextPollInterval(interval, succeeded = false); interval }
        assertEquals(listOf(6L, 12L, 24L, 30L, 30L), seen)
        assertEquals(3L, EnrollmentModel.nextPollInterval(30, succeeded = true))
    }

    @Test
    fun `notConfiguredNeverTouchesTheBackend`() = runTest {
        val model = makeModel(backend = null)
        assertEquals(EnrollmentStatus.NotConfigured("Google sign-in is not configured"), model.status.value)
        model.signIn()
        model.resume()
        assertEquals(EnrollmentStatus.NotConfigured("Google sign-in is not configured"), model.status.value)
    }

    @Test
    fun `signInPendingShowsTheComparisonCodeOfTheStoredKey`() = runTest {
        sleeper.limit = 0
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.Pending("code(${storedPublicKey()})"), model.status.value)
        assertEquals(1, backend.signInCalls)
    }

    @Test
    fun `cancelledSignInReturnsToSignedOut`() = runTest {
        backend.signInError = GoogleSignInError.Cancelled
        val model = makeModel()
        model.signIn()
        assertEquals(EnrollmentStatus.SignedOut, model.status.value)
    }

    @Test
    fun `failedGoogleSignInShowsAnErrorWithAMessage`() = runTest {
        backend.signInError = GoogleSignInError.Failed("UNREGISTERED_ON_API_CONSOLE")
        val model = makeModel()
        model.signIn()
        val status = model.status.value as EnrollmentStatus.Error
        assertEquals("google_signin_failed", status.message?.code)
    }

    @Test
    fun `approvalUnwrapsAndStoresTheGroupKey`() = runTest {
        backend.throwsWhenCancelled = true // the approved naming runs in the poll, before it is cancelled
        backend.devices = listOf(device("pk-mac"))
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        backend.authReplies += { phoneKey -> approved(FakeCoreNative.wrap("pk-mac", phoneKey, "group-key")) }
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.Approved, model.status.value)
        assertEquals("group-key", secrets.groupKey)
        assertEquals(listOf(3L), sleeper.intervals)
        assertEquals("named while pending, again once approved", listOf("Pixel Test", "Pixel Test"), backend.deviceNames)
    }

    @Test
    fun `approvedWithoutHandoffWaitsForGroupKeyAndNeverGeneratesOne`() = runTest {
        sleeper.limit = 0
        backend.devices = listOf(device("pk-mac"))
        backend.authReplies += { _ -> approved() }
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.WaitingForGroupKey("code(${storedPublicKey()})"), model.status.value)
        assertNull(secrets.groupKey)
    }

    @Test
    fun `noApprovedDesktopNeedsDesktopFirstAndNeverGeneratesAKey`() = runTest {
        backend.authReplies += { _ -> approved() }
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
        assertNull(secrets.groupKey)
        assertNull(model.pollJob)
    }

    @Test
    fun `desktopRequiredAnswerNeedsDesktopFirstWithoutRevoking`() = runTest {
        backend.authReplies += { _ -> throw http(409, """{"error":"desktop_required"}""") }
        val model = makeModel()
        model.signIn()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
        assertNotNull(secrets.deviceSecretKey)
        assertTrue(backend.revokedTargets.isEmpty())
    }

    @Test
    fun `otherConflictsAreNotDesktopRequired`() = runTest {
        backend.authReplies += { _ -> throw http(409, """{"error":"other"}""") }
        val model = makeModel()
        model.signIn()
        assertError(model)
    }

    @Test
    fun `approvedWithNoDesktopRevokesItselfAndDiscardsTheDeviceKey`() = runTest {
        backend.authReplies += approvedNoKeyReply()
        val model = makeModel()
        model.signIn()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
        assertEquals(listOf("phone-id"), backend.revokedTargets)
        assertNull(secrets.deviceSecretKey)
    }

    @Test
    fun `selfRevokeFromThePollPathRunsUncancelled`() = runTest {
        backend.throwsWhenCancelled = true
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        backend.authReplies += approvedNoKeyReply()
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
        assertEquals(listOf("phone-id"), backend.revokedTargets)
        assertNull(secrets.deviceSecretKey)
    }

    @Test
    fun `selfRevokeWithoutAnOwnDeviceRowKeepsTheKey`() = runTest {
        backend.authReplies += { _ -> approved() }
        val model = makeModel()
        model.signIn()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
        assertNotNull(secrets.deviceSecretKey)
        assertTrue(backend.revokedTargets.isEmpty())
    }

    @Test
    fun `failedSelfRevokeKeepsTheKeyAndTheNextSignInRetriesBeforeEnrolling`() = runTest {
        backend.authReplies += approvedNoKeyReply()
        backend.revokeError = offline
        val model = makeModel()
        model.signIn()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
        val keptKey = storedPublicKey()
        assertTrue(backend.revokedTargets.isEmpty())

        backend.revokeError = null
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        sleeper.limit = 0
        model.signIn()
        model.pollJob?.join()
        assertEquals("retried before enrolling", listOf("phone-id"), backend.revokedTargets)
        assertNotEquals("enrolled with a fresh keypair", keptKey, storedPublicKey())
    }

    @Test
    fun `signInAfterSelfRevokeGeneratesAFreshKeypair`() = runTest {
        backend.authReplies += approvedNoKeyReply()
        val model = makeModel()
        model.signIn()
        val oldKey = backend.devices.first().publicKey
        sleeper.limit = 0
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        model.signIn()
        model.pollJob?.join()
        assertNotEquals(oldKey, storedPublicKey())
    }

    // A revoked phone wipes itself

    @Test
    fun `aFailingGoogleSignOutStillWipesTheStore`() = runTest {
        seedApprovedPhone()
        backend.signOutError = IllegalStateException("credential manager")
        backend.authReplies += { _ -> throw revoked }
        val model = makeWipingModel()
        model.resume()
        assertWiped(model)
    }

    @Test
    fun `revokedOnResumeFromApprovedWipes`() = runTest {
        seedApprovedPhone()
        backend.authReplies += { _ -> throw revoked }
        val model = makeWipingModel()
        model.resume()
        assertWiped(model)
        assertNull(model.syncCredentials())
    }

    @Test
    fun `revokedWhilePendingPollWipes`() = runTest {
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        backend.authReplies += { _ -> throw revoked }
        val model = makeWipingModel()
        model.signIn()
        model.pollJob?.join()
        assertWiped(model)
        assertNull(model.pollJob)
    }

    @Test
    fun `revokedWhileWaitingForGroupKeyWipes`() = runTest {
        backend.devices = listOf(device("pk-mac"))
        backend.authReplies += { _ -> approved() }
        backend.authReplies += { _ -> throw revoked }
        val model = makeWipingModel()
        model.signIn()
        assertTrue("got ${model.status.value}", model.status.value is EnrollmentStatus.WaitingForGroupKey)
        model.pollJob?.join()
        assertWiped(model)
    }

    @Test
    fun `otherPermanentErrorsDoNotWipe`() = runTest {
        for (failure in listOf(GoogleSignInError.NotSignedIn, http(401, ""), http(403, "email not allowed"))) {
            wipes = 0
            seedApprovedPhone()
            backend.authReplies += { _ -> throw failure }
            val model = makeWipingModel()
            model.resume()
            assertError(model)
            assertNotNull(secrets.groupKey)
            assertNotNull(secrets.deviceSecretKey)
            assertEquals(0, wipes)
            assertEquals(0, backend.signOutCalls)
        }
    }

    @Test
    fun `aSecondRevokedAnswerDoesNotWipeAgain`() = runTest {
        val key = seedApprovedPhone()
        backend.authReplies += { _ -> throw revoked }
        val model = makeWipingModel()
        model.resume()
        model.wipeRevoked(key)
        assertWiped(model)
    }

    /** Fails toward wiping: with no readable device key there is nothing to be stale against. */
    @Test
    fun `revokedAnswerWithNoReadableDeviceKeyStillWipes`() = runTest {
        seedApprovedPhone()
        val model = makeWipingModel()
        secrets.deleteDeviceSecretKey()
        model.wipeRevoked("any-key")
        assertEquals(EnrollmentStatus.Removed, model.status.value)
        assertEquals(1, wipes)
    }

    /** A revoked answer for an older keypair (in flight across a wipe and a new Sign in) must not wipe the new one. */
    @Test
    fun `aStaleRevokedAnswerDoesNotWipeANewKeypair`() = runTest {
        seedApprovedPhone()
        val model = makeWipingModel()
        model.wipeRevoked("an-older-public-key")
        assertEquals(EnrollmentStatus.Approved, model.status.value)
        assertNotNull(secrets.deviceSecretKey)
        assertEquals(0, wipes)
    }

    @Test
    fun `signInAfterTheWipeMakesANewKeypairAndComesUpPending`() = runTest {
        sleeper.limit = 1
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        backend.authReplies += { _ -> throw revoked }
        val model = makeWipingModel()
        model.signIn()
        val oldKey = storedPublicKey()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.Removed, model.status.value)
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        model.signIn()
        model.pollJob?.join()
        val status = model.status.value
        assertTrue("got $status", status is EnrollmentStatus.Pending)
        assertNotEquals(oldKey, storedPublicKey())
        assertNotEquals("code($oldKey)", (status as EnrollmentStatus.Pending).code)
    }

    /** Review Focus 4: an answer that was in flight when the phone was wiped lands nothing. */
    @Test
    fun `anAnswerInFlightAcrossTheWipeLandsNothing`() = runTest {
        val key = seedApprovedPhone()
        backend.authGate = CompletableDeferred()
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        val model = makeWipingModel()
        val resume = launch { model.resume() }
        runCurrent()
        model.wipeRevoked(key)
        backend.authGate!!.complete(Unit)
        resume.join()
        assertWiped(model)
        assertNull(model.syncCredentials())
        assertNull(model.pollJob)
        assertTrue(backend.deviceNames.isEmpty())
    }

    /** Review Focus 4: an error answer in flight across the wipe (here desktop_required on resume) leaves Removed alone. */
    @Test
    fun `aDesktopRequiredAnswerInFlightAcrossTheWipeLandsNothing`() = runTest {
        val key = seedApprovedPhone()
        backend.authGate = CompletableDeferred()
        backend.authReplies += { _ -> throw http(409, """{"error":"desktop_required"}""") }
        val model = makeWipingModel()
        val resume = launch { model.resume() }
        runCurrent()
        model.wipeRevoked(key)
        backend.authGate!!.complete(Unit)
        resume.join()
        assertWiped(model)
    }

    /** Review Focus 4: a sign-in whose request fails after the wipe leaves Removed alone. */
    @Test
    fun `anErrorAnswerInFlightAcrossTheWipeLandsNothing`() = runTest {
        backend.authGate = CompletableDeferred()
        backend.authReplies += { _ -> throw http(409, """{"error":"other"}""") }
        val model = makeWipingModel()
        val signIn = launch { model.signIn() }
        runCurrent()
        model.wipeRevoked(storedPublicKey())
        backend.authGate!!.complete(Unit)
        signIn.join()
        assertWiped(model)
    }

    private val phoneHolder = LimitHolder("phone-id", "Old Pixel", "android")

    private fun limitReply() = { _: String -> GoogleAuthResult.DeviceLimit("phone", listOf(phoneHolder)) }

    @Test
    fun `deviceLimitForPhoneOffersSwapAndWaitlist`() = runTest {
        backend.authReplies += limitReply()
        val model = makeModel()
        model.signIn()
        assertEquals(EnrollmentStatus.DeviceLimit("phone", listOf(phoneHolder)), model.status.value)
        assertNull(model.pollJob)
        assertFalse(model.swapRequested.value)
        assertEquals(
            listOf("more_devices", "routines_while_closed", "phone_notifications", "backups"),
            EnrollmentModel.waitlistOptions.map { it.second },
        )
        assertEquals(
            listOf("More devices", "Routines that run while my computer is closed", "Notifications on my phone", "Backups"),
            EnrollmentModel.waitlistOptions.map { StringCatalog.resolve(it.first, emptyArray()) },
        )
        assertTrue(model.joinWaitlist(listOf("backups")))
        assertEquals(listOf(listOf("backups")), backend.waitlistWants)
        assertNull(model.actionError.value)
    }

    @Test
    fun `deviceLimitCodeRendersCatalogText`() {
        val m = EnrollmentModel.limitMessage(listOf(phoneHolder))
        assertEquals("Whimbrel's free sync includes one computer and one phone. Old Pixel is your synced phone.", m.localized(StringCatalog.resolve))
        val anon = EnrollmentModel.limitMessage(listOf(LimitHolder("x")))
        assertEquals("Whimbrel's free sync includes one computer and one phone. Your other phone is your synced phone.", anon.localized(StringCatalog.resolve))
        assertEquals(anon, EnrollmentModel.limitMessage(emptyList()))
        val core = CoreUserMessage("device_limit", null, "english").localized(StringCatalog.resolve)
        assertEquals("Your free sync plan includes one computer and one phone, and they're already in use.", core)
    }

    @Test
    fun `swapSetsSwapRequestedAndPendingScreenSaysApproveOnYourComputer`() = runTest {
        sleeper.limit = 0
        backend.authReplies += limitReply()
        val model = makeModel()
        model.signIn()
        model.swap(phoneHolder)
        assertEquals(listOf("phone-id"), backend.swappedTargets)
        assertTrue(model.swapRequested.value)
        assertEquals(EnrollmentStatus.Pending("code(${storedPublicKey()})"), model.status.value)
        assertEquals(listOf("Pixel Test"), backend.deviceNames)
        assertNotNull(model.pollJob)
        assertNull(model.actionError.value)
    }

    @Test
    fun `swapFailureSetsActionErrorAndStaysOnTheLimitScreen`() = runTest {
        backend.authReplies += limitReply()
        backend.swapError = http(409, """{"error":"other"}""")
        val model = makeModel()
        model.signIn()
        model.swap(phoneHolder)
        assertFalse(model.swapRequested.value)
        assertTrue(model.status.value is EnrollmentStatus.DeviceLimit)
        assertEquals(CoreUserMessage("from_core", null, "core says no"), model.actionError.value?.core)
    }

    @Test
    fun `waitlistFailureSetsActionError`() = runTest {
        backend.authReplies += limitReply()
        backend.waitlistError = offline
        val model = makeModel()
        model.signIn()
        assertFalse(model.joinWaitlist(listOf("backups")))
        val error = model.actionError.value!!
        assertEquals("core says no", error.localized(StringCatalog.resolve))
        backend.waitlistError = IllegalStateException("boom")
        assertFalse(model.joinWaitlist(listOf("backups")))
        assertEquals("Something went wrong. Try again.", model.actionError.value!!.localized(StringCatalog.resolve))
        backend.waitlistError = null
        assertTrue(model.joinWaitlist(listOf("backups")))
        assertNull(model.actionError.value)
    }

    @Test
    fun `aSwapOrWaitlistErrorInFlightAcrossTheWipeLandsNothing`() = runTest {
        backend.authReplies += limitReply()
        val model = makeWipingModel()
        model.signIn()
        backend.actionGate = CompletableDeferred()
        backend.swapError = http(409, """{"error":"other"}""")
        backend.waitlistError = http(409, """{"error":"other"}""")
        val swap = launch { model.swap(phoneHolder) }
        val waitlist = launch { assertFalse(model.joinWaitlist(listOf("backups"))) }
        runCurrent()
        model.wipeRevoked(storedPublicKey())
        backend.actionGate!!.complete(Unit)
        swap.join()
        waitlist.join()
        assertWiped(model)
        assertNull(model.actionError.value)
    }

    @Test
    fun `aSwapAnsweredAcrossTheWipeLandsNothing`() = runTest {
        backend.authReplies += limitReply()
        val model = makeWipingModel()
        model.signIn()
        backend.actionGate = CompletableDeferred()
        val swap = launch { model.swap(phoneHolder) }
        runCurrent()
        model.wipeRevoked(storedPublicKey())
        backend.actionGate!!.complete(Unit)
        swap.join()
        assertWiped(model)
        assertFalse(model.swapRequested.value)
        assertNull(model.pollJob)
    }

    @Test
    fun `twoSyncsWithin5MinutesListDevicesOnce`() = runTest {
        seedApprovedPhone()
        backend.devices = listOf(device("pk-seed", id = "me").copy(name = "My Pixel"), device("pk-mac"))
        val model = makeModel()
        model.refreshDeviceLabels()
        model.refreshDeviceLabels()
        assertEquals(1, backend.listDevicesCalls)
        assertEquals(DeviceLabels(mapOf("pk-seed" to "My Pixel", "pk-mac" to "Mac"), "pk-seed"), model.deviceLabels.value)
        now += 299_000
        model.refreshDeviceLabels()
        assertEquals(1, backend.listDevicesCalls)
        now += 2_000
        model.refreshDeviceLabels()
        assertEquals(2, backend.listDevicesCalls)
        model.enterForeground()
        model.refreshDeviceLabels()
        assertEquals(3, backend.listDevicesCalls)
    }

    @Test
    fun `refreshDeviceLabelsStaysQuietWhenNotApprovedOrOffline`() = runTest {
        val signedOut = makeModel()
        signedOut.refreshDeviceLabels()
        assertEquals(0, backend.listDevicesCalls)
        seedApprovedPhone()
        backend.idTokenError = GoogleSignInError.NotSignedIn
        val model = makeModel()
        model.refreshDeviceLabels()
        assertEquals(DeviceLabels(), model.deviceLabels.value)
    }

    @Test
    fun `wrappedKeyFromANonApprovedSenderStoresNothing`() = runTest {
        sleeper.limit = 0
        backend.devices = listOf(device("pk-stranger", status = "pending"))
        backend.authReplies += { phoneKey -> approved(FakeCoreNative.wrap("pk-stranger", phoneKey, "group-key")) }
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.WaitingForGroupKey("code(${storedPublicKey()})"), model.status.value)
        assertNull(secrets.groupKey)
    }

    @Test
    fun `permanentFailureWhilePollingStopsWithAnError`() = runTest {
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        backend.authReplies += { _ -> throw http(403, "") }
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertError(model)
        assertEquals(listOf(3L), sleeper.intervals)
    }

    @Test
    fun `transientFailureBacksOffThenResets`() = runTest {
        sleeper.limit = 3
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        backend.authReplies += { _ -> throw http(502, "") }
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        assertEquals(listOf(3L, 6L, 3L, 3L), sleeper.intervals)
        assertTrue("got ${model.status.value}", model.status.value is EnrollmentStatus.Pending)
    }

    @Test
    fun `resumeWithoutAKeypairStaysSignedOut`() = runTest {
        val model = makeModel()
        model.resume()
        assertEquals(EnrollmentStatus.SignedOut, model.status.value)
        assertEquals(0, backend.authCalls)
    }

    @Test
    fun `resumeWithAKeypairPicksUpPending`() = runTest {
        sleeper.limit = 0
        secrets.deviceSecretKey = "sk-77"
        val model = makeModel()
        model.resume()
        model.pollJob?.join()
        assertEquals(EnrollmentStatus.Pending("code(pk-77)"), model.status.value)
        assertEquals(0, backend.signInCalls)
    }

    // Offline launch, credentials, renewal, background

    @Test
    fun `launchWithAGroupKeyStartsApprovedEvenOffline`() = runTest {
        seedApprovedPhone()
        backend.authReplies += { _ -> throw offline }
        val model = makeModel()
        assertEquals(EnrollmentStatus.Approved, model.status.value)
        model.resume()
        assertEquals(EnrollmentStatus.Approved, model.status.value)
        assertNull("no Turso token until the Worker answers", model.syncCredentials())
    }

    @Test
    fun `approvedResumeKeepsTheTursoTokenInMemoryForSync`() = runTest {
        val publicKey = seedApprovedPhone()
        backend.authReplies += { _ -> GoogleAuthResult.Approved("turso-token", "libsql://db", null) }
        val model = makeModel()
        model.resume()
        val credentials = model.syncCredentials()!!
        assertEquals("turso-token", credentials.tursoToken)
        assertEquals("libsql://db", credentials.tursoDatabaseUrl)
        assertEquals(secrets.groupKey, credentials.groupKeyBase64)
        assertEquals("the device id is the device public key, as on the Mac", publicKey, credentials.deviceId)
        assertFalse("never printable", credentials.toString().contains("turso-token"))
        assertFalse(credentials.toString().contains("libsql://db"))
    }

    @Test
    fun `revokedOnResumeWhileApprovedIsAnError`() = runTest {
        seedApprovedPhone()
        backend.authReplies += { _ -> throw http(403, "") }
        val model = makeModel()
        model.resume()
        assertError(model)
        assertNull(model.syncCredentials())
    }

    /** The "Sign in again" state: the core's `signin_expired` message, which the screen pairs with Sign in. */
    @Test
    fun `notSignedInWhilePollingIsPermanent`() = runTest {
        backend.authReplies += { _ ->
            backend.idTokenError = GoogleSignInError.NotSignedIn
            GoogleAuthResult.Pending
        }
        val model = makeModel()
        model.signIn()
        model.pollJob?.join()
        val status = model.status.value
        assertTrue("got $status", status is EnrollmentStatus.Error)
        assertEquals("signin_expired", (status as EnrollmentStatus.Error).message?.code)
        assertEquals("stops instead of backing off forever", listOf(3L), sleeper.intervals)
        assertNull(model.pollJob)
    }

    @Test
    fun `backgroundStopsPollingAndForegroundResumesIt`() = runTest {
        // A poll sleep that only ends when cancelled.
        val model = makeModel(sleep = { awaitCancellation() })
        model.signIn()
        assertTrue("got ${model.status.value}", model.status.value is EnrollmentStatus.Pending)
        val poll = model.pollJob!!
        model.enterBackground()
        assertNull(model.pollJob)
        assertTrue("the poll must be cancelled", poll.isCancelled)
        poll.join()
        assertEquals("no poll while in the background", 1, backend.authCalls)

        model.enterForeground()
        assertEquals("re-checks at once on foreground", 2, backend.authCalls)
        assertNotNull("and polls again", model.pollJob)
        model.enterBackground()
    }

    @Test
    fun `foregroundWhenApprovedDoesNotPoll`() = runTest {
        seedApprovedPhone()
        val model = makeModel()
        model.enterForeground()
        assertNull(model.pollJob)
        assertEquals(0, backend.authCalls)
    }

    @Test
    fun `pendingAndApprovedNameTheDeviceBestEffort`() = runTest {
        backend.setNameError = offline
        backend.devices = listOf(device("pk-mac"))
        backend.authReplies += { _ -> GoogleAuthResult.Pending }
        backend.authReplies += { phoneKey -> approved(FakeCoreNative.wrap("pk-mac", phoneKey, "group-key")) }
        val model = makeModel()
        model.signIn()
        assertTrue("a failed name keeps pending", model.status.value is EnrollmentStatus.Pending)
        model.pollJob?.join()
        assertEquals("a failed name keeps approved", EnrollmentStatus.Approved, model.status.value)
        assertEquals(listOf("Pixel Test", "Pixel Test"), backend.deviceNames)
    }

    @Test
    fun `renewIfDueReauthenticatesOnceWithinTheFloor`() = runTest {
        seedApprovedPhone()
        repeat(3) { backend.authReplies += { _ -> approved() } }
        val model = makeModel()
        model.renewIfDue(afterFailure = false, force = false)
        assertEquals("no token yet: renew", 1, backend.authCalls)
        now += 60_000
        model.renewIfDue(afterFailure = true, force = false)
        assertEquals("inside the 5-minute floor", 1, backend.authCalls)
        model.renewIfDue(afterFailure = true, force = true)
        assertEquals("Sync now forces it", 2, backend.authCalls)
    }

    @Test
    fun `aRestartReusesTheSavedTokenWithoutAskingGoogle`() = runTest {
        seedApprovedPhone()
        backend.authReplies += { _ -> approved() }
        makeModel().renewIfDue(afterFailure = false, force = true)
        assertEquals(1, backend.authCalls)
        assertNotNull("saved for the next process", secrets.tursoSession)

        // A new process: the token comes from the vault, so launching asks Google for nothing.
        now += 60 * 60_000
        val restarted = makeModel()
        restarted.launch()
        assertEquals("no Google call on a restart", 1, backend.authCalls)
        assertNotNull(restarted.syncCredentials())
        // Twelve hours after it was issued, the token is renewed as before.
        backend.authReplies += { _ -> approved() }
        now += 12 * 60 * 60_000
        restarted.renewIfDue(afterFailure = false, force = false)
        assertEquals(2, backend.authCalls)
    }

    @Test
    fun `aWipeDeletesTheSavedToken`() = runTest {
        seedApprovedPhone()
        backend.authReplies += { _ -> approved() }
        backend.authReplies += { _ -> throw revoked }
        val model = makeWipingModel()
        model.renewIfDue(afterFailure = false, force = true)
        assertNotNull(secrets.tursoSession)
        model.renewIfDue(afterFailure = true, force = true)
        assertNull(secrets.tursoSession)
    }

    /** A poll request in flight when the app goes to the background must not start a new poll when it returns. */
    @Test
    fun `backgroundDuringAnInFlightPollRequestStartsNoNewPoll`() = runTest {
        backend.blockingAuthCall = 2
        val model = makeModel()
        model.signIn()
        val poll = model.pollJob!!
        runCurrent()
        assertEquals(2, backend.authCalls)
        model.enterBackground()
        poll.join()
        assertNull("no poll restarted by the late answer", model.pollJob)
        assertEquals(2, backend.authCalls)
        assertEquals("no further poll ticks", listOf(3L), sleeper.intervals)
    }

    /** A permanent error thrown by a request that was cancelled meanwhile (app backgrounded) must not change state. */
    @Test
    fun `permanentErrorAfterCancellationDoesNotChangeStatus`() = runTest {
        backend.blockingAuthCall = 2
        backend.blockingAuthError = http(403, "")
        val model = makeModel()
        model.signIn()
        val before = model.status.value
        val poll = model.pollJob!!
        runCurrent()
        assertEquals(2, backend.authCalls)
        model.enterBackground()
        poll.join()
        assertEquals(before, model.status.value)
    }

    // Android: silent renewal through Credential Manager (Review Focus 5)

    /** Offline, or Play services unavailable for the silent request: an approved phone stays approved, no error. */
    @Test
    fun `offlineResumeKeepsApprovedAndShowsNoError`() = runTest {
        seedApprovedPhone()
        backend.authReplies += { _ -> throw offline }
        val model = makeWipingModel()
        model.resume()
        assertEquals(EnrollmentStatus.Approved, model.status.value)

        backend.idTokenError = GoogleSignInError.Failed("network")
        model.renewIfDue(afterFailure = true, force = true)
        assertEquals(EnrollmentStatus.Approved, model.status.value)
        assertNotNull(secrets.groupKey)
        assertEquals(0, wipes)
    }

    /**
     * Auto-select hands back a Google account with no Whimbrel desktop (on sign-in, or on a silent
     * renewal while approved): the Worker answers 409 desktop_required, which says "Set up Whimbrel on
     * your Mac first", not an error, and wipes nothing.
     */
    @Test
    fun `wrongGoogleAccountWithoutADesktopSaysSetUpTheMacFirst`() = runTest {
        val desktopRequired = http(409, """{"error":"desktop_required"}""")
        seedApprovedPhone()
        backend.authReplies += { _ -> throw desktopRequired }
        val model = makeWipingModel()
        model.resume()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
        assertNull("no sync with another account's token", model.syncCredentials())
        assertNotNull(secrets.groupKey)
        assertNotNull(secrets.deviceSecretKey)
        assertEquals(0, wipes)

        backend.authReplies += { _ -> throw desktopRequired }
        model.signIn()
        assertEquals(EnrollmentStatus.NeedsDesktopFirst, model.status.value)
    }
}
