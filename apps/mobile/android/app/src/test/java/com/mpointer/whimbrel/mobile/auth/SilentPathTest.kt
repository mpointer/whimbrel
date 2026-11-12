package com.mpointer.whimbrel.mobile.auth

import com.mpointer.whimbrel.mobile.sync.PrefStore
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import kotlinx.coroutines.runBlocking
import org.junit.Test

private class MapPrefs : PrefStore {
    val m = mutableMapOf<String, Any>()
    override fun getString(key: String) = m[key] as? String
    override fun getLong(key: String) = m[key] as? Long
    override fun getBoolean(key: String) = m[key] as? Boolean ?: false
    override fun putString(key: String, v: String) { m[key] = v }
    override fun putLong(key: String, v: Long) { m[key] = v }
    override fun putBoolean(key: String, v: Boolean) { m[key] = v }
    override fun clearPrefix(prefix: String) { m.keys.removeAll { it.startsWith(prefix) } }
}

class SilentPathTest {
    @Test fun noRememberedEmailUsesCredentialManager() {
        assertSame(SilentPath.CredentialManager, silentPathFor(null))
        assertSame(SilentPath.CredentialManager, silentPathFor(""))
        assertSame(SilentPath.CredentialManager, silentPathFor("   "))
    }

    @Test fun rememberedEmailUsesLegacyForThatAccount() {
        assertEquals(SilentPath.LegacyForAccount("a@b.com"), silentPathFor("a@b.com"))
    }

    private fun failed(o: LegacyOutcome) = (o as? LegacyOutcome.Fail)?.error is GoogleSignInError.Failed

    @Test fun signInRequiredNeedsTheFallbackNotASignOut() {
        assertSame(LegacyOutcome.TryCredentialManager, classifyLegacyFailure(LegacyStatus.SIGN_IN_REQUIRED, online = true))
    }

    @Test fun cancelledIsTransient() {
        assertTrue(failed(classifyLegacyFailure(LegacyStatus.SIGN_IN_CANCELLED, online = true)))
    }

    @Test fun offlineIsAlwaysTransient() {
        listOf(LegacyStatus.SIGN_IN_REQUIRED, LegacyStatus.NETWORK_ERROR, LegacyStatus.INTERNAL_ERROR, LegacyStatus.SIGN_IN_CANCELLED)
            .forEach { assertTrue("$it", failed(classifyLegacyFailure(it, online = false))) }
    }

    @Test fun otherStatusesAreTransient() {
        listOf(LegacyStatus.NETWORK_ERROR, LegacyStatus.INTERNAL_ERROR, LegacyStatus.TIMEOUT, LegacyStatus.DEVELOPER_ERROR, LegacyStatus.SIGN_IN_FAILED, 999)
            .forEach { assertTrue("$it", failed(classifyLegacyFailure(it, online = true))) }
    }

    @Test fun legacyRequiredFallsBackToCredentialManager() = runBlocking {
        var cmCalls = 0
        val token = renewLegacyThenFallback({ LegacyResult.TryCredentialManager }) { cmCalls++; "cm-token" }
        assertEquals("cm-token", token)
        assertEquals(1, cmCalls)
    }

    @Test fun legacyTokenSkipsCredentialManager() = runBlocking {
        val token = renewLegacyThenFallback({ LegacyResult.Token("legacy") }) { error("must not be called") }
        assertEquals("legacy", token)
    }

    @Test fun notSignedInOnlyWhenBothPathsSaySo() = runBlocking {
        suspend fun run(legacy: LegacyResult?, legacyError: GoogleSignInError?, cm: () -> String): Throwable? = try {
            renewLegacyThenFallback({ legacyError?.let { throw it }; legacy!! }) { cm() }; null
        } catch (e: GoogleSignInError) { e }
        // both say not signed in
        assertSame(GoogleSignInError.NotSignedIn, run(LegacyResult.TryCredentialManager, null) { throw GoogleSignInError.NotSignedIn })
        // legacy needs the fallback, Credential Manager is transient (two accounts: chooser timeout)
        assertTrue(run(LegacyResult.TryCredentialManager, null) { throw GoogleSignInError.Failed("timeout") } is GoogleSignInError.Failed)
        // legacy transient failure never reaches Credential Manager or becomes NotSignedIn
        assertTrue(run(null, GoogleSignInError.Failed("net")) { throw GoogleSignInError.NotSignedIn } is GoogleSignInError.Failed)
    }

    @Test fun storeRemembersAndSignOutClearsIt() {
        val prefs = MapPrefs()
        val store = AccountHintStore(prefs)
        assertNull(store.get())
        store.set("mike@example.com")
        assertEquals("mike@example.com", store.get())
        assertEquals("mike@example.com", AccountHintStore(prefs).get())
        store.clear()
        assertNull(store.get())
        assertTrue(prefs.m.isEmpty())
    }

    @Test fun blankEmailIsNotRemembered() {
        val store = AccountHintStore(MapPrefs())
        store.set("  ")
        assertNull(store.get())
    }
}
