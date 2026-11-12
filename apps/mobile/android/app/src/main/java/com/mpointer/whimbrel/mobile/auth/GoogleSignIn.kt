package com.mpointer.whimbrel.mobile.auth

import android.app.Activity
import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log
import androidx.credentials.ClearCredentialStateRequest
import androidx.credentials.CredentialManager
import androidx.credentials.CustomCredential
import androidx.credentials.GetCredentialRequest
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialInterruptedException
import androidx.credentials.exceptions.GetCredentialProviderConfigurationException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.credentials.exceptions.NoCredentialException
import com.google.android.libraries.identity.googleid.GetGoogleIdOption
import com.google.android.libraries.identity.googleid.GetSignInWithGoogleOption
import com.google.android.libraries.identity.googleid.GoogleIdTokenCredential
import com.google.android.libraries.identity.googleid.GoogleIdTokenParsingException
import android.accounts.Account
import com.google.android.gms.auth.GoogleAuthException
import com.google.android.gms.auth.GoogleAuthUtil
import com.google.android.gms.auth.UserRecoverableAuthException
import com.google.android.gms.auth.api.signin.GoogleSignInOptions
import kotlinx.coroutines.withContext
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.tasks.Task
import com.mpointer.whimbrel.mobile.core.CoreNative
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

sealed class GoogleSignInError : Exception() {
    object Cancelled : GoogleSignInError()
    object NotSignedIn : GoogleSignInError()
    class Failed(val reason: String) : GoogleSignInError() {
        override val message get() = reason
    }
}

/**
 * Silent renewal has no UI, so anything that needs the user (no account, a chooser the application
 * context cannot show, unsupported) is permanent: sign in again. Network, Play services and unknown
 * trouble is transient, and so is "no credential" while the phone is offline.
 */
internal fun classifySilentFailure(e: GetCredentialException, online: Boolean): GoogleSignInError = when {
    e is GetCredentialInterruptedException || e is GetCredentialProviderConfigurationException
        || e is GetCredentialUnknownException -> GoogleSignInError.Failed(e.message ?: e.type)
    // Offline, Credential Manager also says "no credential": only a phone with a working network has lost it.
    e is NoCredentialException && !online -> GoogleSignInError.Failed(e.message ?: e.type)
    else -> GoogleSignInError.NotSignedIn
}

/** A validated network: offline is never a reason to sign in again. */
private fun Context.isOnline(): Boolean {
    val cm = getSystemService(ConnectivityManager::class.java) ?: return false
    val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
    return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
}

/**
 * An interactive failure. Only a real user dismissal is silent; Google reports some provider faults
 * (an unregistered OAuth client) as a cancellation with a different message, and those must show an error.
 */
internal fun classifyInteractiveFailure(e: GetCredentialException): GoogleSignInError =
    if (e is GetCredentialCancellationException && (e.message.isNullOrBlank() || e.message!!.contains("cancel", ignoreCase = true))) {
        GoogleSignInError.Cancelled
    } else {
        GoogleSignInError.Failed(e.message ?: e.type)
    }

/** Google sign-in through Credential Manager: interactive once, then silent renewal behind [cache]. */
/** How long silent renewal waits for Credential Manager before giving up (transient). */
internal const val SILENT_TIMEOUT_MS = 15_000L

/** Logcat tag for sign-in. Lines name the step and the outcome only: never a token or an email. */
internal const val SIGNIN_TAG = "google-signin"

class GoogleSignIn(
    context: Context,
    private val webClientId: String,
    private val cache: IdTokenCache,
    private val account: AccountHintStore,
) {
    private val appContext = context.applicationContext
    private val manager = CredentialManager.create(appContext)

    suspend fun authorize(activity: Activity) {
        Log.i(SIGNIN_TAG, "interactive sign-in started")
        cache.clear()
        val request = GetCredentialRequest(listOf(GetSignInWithGoogleOption.Builder(webClientId).build()))
        try {
            // Seed the cache: else the next step asks Credential Manager again, a second chooser.
            val credential = manager.getCredential(activity, request).credential
            val token = idToken(credential)
            // Remember which account was chosen, so silent renewal can target it (the email is `id`).
            emailOf(credential)?.let(account::set)
            cache.seed(token)
            Log.i(SIGNIN_TAG, "interactive sign-in ok; account remembered=${account.get() != null}")
        } catch (e: GetCredentialException) {
            // No token is ever in these; the message names the cause (e.g. UNREGISTERED_ON_API_CONSOLE).
            Log.w("google-signin", "sign-in failed: ${e.javaClass.simpleName}: ${e.message}")
            throw classifyInteractiveFailure(e)
        }
    }

    suspend fun validIdToken(): String = cache.get()

    /** The cache's fetch: no UI, only an account that already authorized this app. */
    internal suspend fun fetchSilently(): String {
        val path = silentPathFor(account.get())
        Log.i(SIGNIN_TAG, "silent renewal: path=${if (path is SilentPath.LegacyForAccount) "legacy-for-account" else "credential-manager"} online=${appContext.isOnline()}")
        try {
            val token = when (path) {
                is SilentPath.LegacyForAccount -> fetchForAccount(path.email)
                    ?: renewLegacyThenFallback({ fetchLegacy(path.email) }, ::fetchViaCredentialManager)
                SilentPath.CredentialManager -> fetchViaCredentialManager()
            }
            Log.i(SIGNIN_TAG, "silent renewal: ok")
            return token
        } catch (e: CancellationException) {
            Log.i(SIGNIN_TAG, "silent renewal: cancelled")
            throw e
        } catch (e: GoogleSignInError) {
            Log.w(SIGNIN_TAG, "silent renewal: failed as ${describe(e)}")
            throw e
        }
    }

    private fun describe(e: GoogleSignInError): String = when (e) {
        GoogleSignInError.NotSignedIn -> "NotSignedIn (asks to sign in again)"
        GoogleSignInError.Cancelled -> "Cancelled"
        is GoogleSignInError.Failed -> "Failed (transient): ${e.reason}"
    }

    /**
     * An ID token for exactly [email], with no UI, from Play services' account token API. Unlike the
     * legacy sign-in API it works whichever API the user consented through, so a phone with two Google
     * accounts never sees a chooser. Null means "try the other paths": consent is needed (no UI here)
     * or Play services refused. Network trouble is transient.
     */
    private suspend fun fetchForAccount(email: String): String? = withContext(Dispatchers.IO) {
        val scope = "audience:server:client_id:$webClientId"
        try {
            val token = withTimeoutOrNull(SILENT_TIMEOUT_MS) {
                GoogleAuthUtil.getToken(appContext, Account(email, "com.google"), scope)
            } ?: run {
                Log.w(SIGNIN_TAG, "account token: timed out after ${SILENT_TIMEOUT_MS}ms")
                throw GoogleSignInError.Failed("silent renewal timed out")
            }
            // Play services caches the token; drop its copy so the next renewal gets a fresh one
            // (our own cache holds it until it expires).
            runCatching { GoogleAuthUtil.clearToken(appContext, token) }
            Log.i(SIGNIN_TAG, "account token: success")
            token
        } catch (e: CancellationException) {
            throw e
        } catch (e: GoogleSignInError) {
            throw e
        } catch (e: UserRecoverableAuthException) {
            Log.w(SIGNIN_TAG, "account token: needs consent (UserRecoverableAuthException) -> other paths")
            null
        } catch (e: GoogleAuthException) {
            Log.w(SIGNIN_TAG, "account token: ${e.javaClass.simpleName} -> other paths")
            null
        } catch (e: java.io.IOException) {
            Log.w(SIGNIN_TAG, "account token: IOException")
            throw GoogleSignInError.Failed(e.message ?: "network")
        }
    }

    /**
     * Renews for exactly the remembered account, with no UI. The legacy API is deprecated but is the only one
     * that can name an account. The token's audience is [webClientId], same as the Credential Manager path.
     */
    @Suppress("DEPRECATION")
    private suspend fun fetchLegacy(email: String): LegacyResult {
        val client = legacyClient(email)
        try {
            val result = withTimeoutOrNull(SILENT_TIMEOUT_MS) { client.silentSignIn().await() }
                ?: run {
                    Log.w(SIGNIN_TAG, "legacy: timed out after ${SILENT_TIMEOUT_MS}ms")
                    throw GoogleSignInError.Failed("silent renewal timed out")
                }
            Log.i(SIGNIN_TAG, "legacy: success, idToken=${if (result.idToken != null) "present" else "missing -> credential manager"}")
            return result.idToken?.let { LegacyResult.Token(it) } ?: LegacyResult.TryCredentialManager
        } catch (e: CancellationException) {
            throw e
        } catch (e: GoogleSignInError) {
            throw e
        } catch (e: ApiException) {
            Log.w(SIGNIN_TAG, "legacy: ApiException status=${e.statusCode}")
            when (val o = classifyLegacyFailure(e.statusCode, appContext.isOnline())) {
                LegacyOutcome.TryCredentialManager -> return LegacyResult.TryCredentialManager
                is LegacyOutcome.Fail -> throw o.error
            }
        } catch (e: Exception) {
            Log.w(SIGNIN_TAG, "legacy: ${e.javaClass.simpleName}")
            throw GoogleSignInError.Failed(e.message ?: e.javaClass.simpleName)
        }
    }

    @Suppress("DEPRECATION")
    private fun legacyClient(email: String?) = com.google.android.gms.auth.api.signin.GoogleSignIn.getClient(
        appContext,
        GoogleSignInOptions.Builder(GoogleSignInOptions.DEFAULT_SIGN_IN).requestIdToken(webClientId).requestEmail()
            .apply { if (email != null) setAccountName(email) }.build(),
    )

    private suspend fun fetchViaCredentialManager(): String {
        val option = GetGoogleIdOption.Builder()
            .setServerClientId(webClientId)
            .setFilterByAuthorizedAccounts(true)
            .setAutoSelectEnabled(true)
            .build()
        try {
            // Never wait on a chooser nobody sees: from the background Credential Manager opens it hidden,
            // which held every sync round. Cancelling dismisses it; the round keeps its current token.
            val response = withTimeoutOrNull(SILENT_TIMEOUT_MS) {
                manager.getCredential(appContext, GetCredentialRequest(listOf(option)))
            } ?: run {
                Log.w(SIGNIN_TAG, "credential manager: timed out after ${SILENT_TIMEOUT_MS}ms")
                throw GoogleSignInError.Failed("silent renewal timed out")
            }
            val token = idToken(response.credential)
            // Installs from before the account was remembered get here every time, and with two
            // authorized Google accounts Credential Manager shows its chooser. Remember the account
            // it returned, so later renewals name it on the legacy path and need no chooser.
            val remembered = account.get() == null && emailOf(response.credential)?.also(account::set) != null
            Log.i(SIGNIN_TAG, "credential manager: success; account newly remembered=$remembered")
            return token
        } catch (e: GetCredentialException) {
            // The type and message name the cause (e.g. NoCredentialException); neither holds a token.
            Log.w(SIGNIN_TAG, "credential manager: ${e.javaClass.simpleName} type=${e.type} msg=${e.message}")
            throw classifySilentFailure(e, appContext.isOnline())
        }
    }

    /** Wipe only; there is no sign-out control. Never blocks: on Android 10-13 clearing Google's state
     *  answers on the main looper, so blocking the main thread for it deadlocks. */
    fun signOut() {
        cache.clear()
        account.clear()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                manager.clearCredentialState(ClearCredentialStateRequest())
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
            }
            try {
                legacyClient(null).signOut()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Nothing signed in, or Play services unavailable: the cache is already cleared.
            }
        }
    }

    private fun emailOf(credential: androidx.credentials.Credential): String? {
        if (credential is CustomCredential && credential.type == GoogleIdTokenCredential.TYPE_GOOGLE_ID_TOKEN_CREDENTIAL) {
            return try { GoogleIdTokenCredential.createFrom(credential.data).id } catch (_: GoogleIdTokenParsingException) { null }
        }
        return null
    }

    private fun idToken(credential: androidx.credentials.Credential): String {
        if (credential is CustomCredential && credential.type == GoogleIdTokenCredential.TYPE_GOOGLE_ID_TOKEN_CREDENTIAL) {
            try {
                return GoogleIdTokenCredential.createFrom(credential.data).idToken
            } catch (e: GoogleIdTokenParsingException) {
                throw GoogleSignInError.Failed(e.message ?: "bad token")
            }
        }
        throw GoogleSignInError.Failed("unexpected credential type")
    }

    companion object {
        /** Builds the cache around this instance's silent fetch (they need each other). */
        fun create(context: Context, webClientId: String, native: CoreNative, account: AccountHintStore): GoogleSignIn {
            lateinit var google: GoogleSignIn
            val cache = IdTokenCache(System::currentTimeMillis, { google.fetchSilently() }) { jwt ->
                runCatching {
                    Json.parseToJsonElement(native.oauthJwtClaims(jwt)!!).jsonObject["exp"]?.jsonPrimitive?.longOrNull
                }.getOrNull()
            }
            google = GoogleSignIn(context, webClientId, cache, account)
            return google
        }
    }
}

/** Awaits a Play services Task without another dependency; cancelling the coroutine abandons the result. */
private suspend fun <T> Task<T>.await(): T = suspendCancellableCoroutine { cont ->
    addOnSuccessListener { cont.resume(it) }
    addOnFailureListener { cont.resumeWithException(it) }
    addOnCanceledListener { cont.cancel() }
}
