package com.mpointer.whimbrel.mobile.auth

import com.google.android.gms.auth.api.signin.GoogleSignInStatusCodes
import com.google.android.gms.common.api.CommonStatusCodes
import com.mpointer.whimbrel.mobile.sync.PrefStore

/** How silent renewal reaches Google: the account the user chose, or (installs before that was remembered) Credential Manager. */
sealed class SilentPath {
    data class LegacyForAccount(val email: String) : SilentPath()
    object CredentialManager : SilentPath()
}

internal fun silentPathFor(rememberedEmail: String?): SilentPath =
    rememberedEmail?.trim()?.takeIf { it.isNotEmpty() }?.let { SilentPath.LegacyForAccount(it) } ?: SilentPath.CredentialManager

/** The legacy sign-in API's status codes that matter here. */
internal object LegacyStatus {
    const val SIGN_IN_REQUIRED = CommonStatusCodes.SIGN_IN_REQUIRED
    const val NETWORK_ERROR = CommonStatusCodes.NETWORK_ERROR
    const val INTERNAL_ERROR = CommonStatusCodes.INTERNAL_ERROR
    const val TIMEOUT = CommonStatusCodes.TIMEOUT
    const val DEVELOPER_ERROR = CommonStatusCodes.DEVELOPER_ERROR
    const val SIGN_IN_FAILED = GoogleSignInStatusCodes.SIGN_IN_FAILED
    const val SIGN_IN_CANCELLED = GoogleSignInStatusCodes.SIGN_IN_CANCELLED
}

/** What a failed legacy call means: ask Credential Manager next, or give up transiently. Never a sign-out by itself. */
internal sealed class LegacyOutcome {
    object TryCredentialManager : LegacyOutcome()
    class Fail(val error: GoogleSignInError) : LegacyOutcome()
}

/** A legacy call that succeeded: a token, or no usable token (then Credential Manager decides). */
internal sealed class LegacyResult {
    class Token(val idToken: String) : LegacyResult()
    object TryCredentialManager : LegacyResult()
}

/**
 * The legacy API may hold no grant for an account whose consent came from Credential Manager, so
 * "sign-in required" only means ask Credential Manager. Offline and every other status (including a
 * "cancelled" silent call) is transient.
 */
internal fun classifyLegacyFailure(statusCode: Int, online: Boolean): LegacyOutcome = when {
    !online -> LegacyOutcome.Fail(GoogleSignInError.Failed("offline (status $statusCode)"))
    statusCode == LegacyStatus.SIGN_IN_REQUIRED -> LegacyOutcome.TryCredentialManager
    else -> LegacyOutcome.Fail(GoogleSignInError.Failed("silent sign-in status $statusCode"))
}

/**
 * Legacy first; if it needs the fallback, Credential Manager decides. NotSignedIn comes only from
 * Credential Manager, so it is only thrown when both paths say so. [legacy] throws [GoogleSignInError.Failed] for transient trouble.
 */
internal suspend fun renewLegacyThenFallback(legacy: suspend () -> LegacyResult, credentialManager: suspend () -> String): String =
    when (val r = legacy()) {
        is LegacyResult.Token -> r.idToken
        LegacyResult.TryCredentialManager -> credentialManager()
    }

/** The email of the account the user chose at interactive sign-in. Cleared on sign-out. */
class AccountHintStore(private val prefs: PrefStore) {
    fun get(): String? = prefs.getString(KEY)?.takeIf { it.isNotBlank() }
    fun set(email: String) { if (email.isNotBlank()) prefs.putString(KEY, email.trim()) }
    fun clear() = prefs.clearPrefix(KEY)

    private companion object { const val KEY = "google.account.email" }
}
