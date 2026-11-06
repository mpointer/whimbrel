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
import com.mpointer.whimbrel.mobile.core.CoreNative
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

class GoogleSignIn(context: Context, private val webClientId: String, private val cache: IdTokenCache) {
    private val appContext = context.applicationContext
    private val manager = CredentialManager.create(appContext)

    suspend fun authorize(activity: Activity) {
        cache.clear()
        val request = GetCredentialRequest(listOf(GetSignInWithGoogleOption.Builder(webClientId).build()))
        try {
            // Seed the cache: else the next step asks Credential Manager again, a second chooser.
            cache.seed(idToken(manager.getCredential(activity, request).credential))
        } catch (e: GetCredentialException) {
            // No token is ever in these; the message names the cause (e.g. UNREGISTERED_ON_API_CONSOLE).
            Log.w("google-signin", "sign-in failed: ${e.javaClass.simpleName}: ${e.message}")
            throw classifyInteractiveFailure(e)
        }
    }

    suspend fun validIdToken(): String = cache.get()

    /** The cache's fetch: no UI, only an account that already authorized this app. */
    internal suspend fun fetchSilently(): String {
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
            } ?: throw GoogleSignInError.Failed("silent renewal timed out")
            return idToken(response.credential)
        } catch (e: GetCredentialException) {
            throw classifySilentFailure(e, appContext.isOnline())
        }
    }

    /** Wipe only; there is no sign-out control. Never blocks: on Android 10-13 clearing Google's state
     *  answers on the main looper, so blocking the main thread for it deadlocks. */
    fun signOut() {
        cache.clear()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                manager.clearCredentialState(ClearCredentialStateRequest())
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Nothing signed in, or Play services unavailable: the cache is already cleared.
            }
        }
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
        fun create(context: Context, webClientId: String, native: CoreNative): GoogleSignIn {
            lateinit var google: GoogleSignIn
            val cache = IdTokenCache(System::currentTimeMillis, { google.fetchSilently() }) { jwt ->
                runCatching {
                    Json.parseToJsonElement(native.oauthJwtClaims(jwt)!!).jsonObject["exp"]?.jsonPrimitive?.longOrNull
                }.getOrNull()
            }
            google = GoogleSignIn(context, webClientId, cache)
            return google
        }
    }
}
