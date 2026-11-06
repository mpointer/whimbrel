package com.mpointer.whimbrel.mobile.ui

import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.core.CoreUserMessage
import com.mpointer.whimbrel.mobile.core.LimitHolder
import com.mpointer.whimbrel.mobile.sync.EnrollmentStatus
import com.mpointer.whimbrel.mobile.sync.StringCatalog
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EnrollmentContentTest {
    private val resolve = StringCatalog.resolve
    private val code = "K7 · 4Q · 92 · 9K · 3M"

    @Test
    fun pendingShowsTheCodeAndTheApproveOnYourMacCopy() {
        val c = enrollmentContent(EnrollmentStatus.Pending(code), swapRequested = false)
        assertEquals(R.string.enroll_waiting_title, c.titleRes)
        assertEquals(R.string.enroll_pending_detail, c.bodyRes)
        assertEquals(code, c.code)
        assertTrue(c.showsProgress)
        assertTrue(c.actions.isEmpty())
        assertEquals(
            "On your Mac, open Whimbrel Settings → Devices and approve this phone. Check that the code matches.",
            resolve(c.bodyRes!!, emptyArray()),
        )
    }

    @Test
    fun pendingAfterSwapSaysApproveOnYourComputer() {
        val c = enrollmentContent(EnrollmentStatus.Pending(code), swapRequested = true)
        assertEquals("Approve this phone on your computer.", resolve(c.bodyRes!!, emptyArray()))
    }

    @Test
    fun codeDescriptionSpellsTheCodeWithoutSeparators() {
        val c = enrollmentContent(EnrollmentStatus.Pending(code), swapRequested = false)
        assertEquals("Comparison code K7 4Q 92 9K 3M", c.codeDescription!!.localized(resolve))
        assertEquals("Comparison code K7 4Q 92 9K 3M", enrollmentContent(EnrollmentStatus.WaitingForGroupKey(code), false).codeDescription!!.localized(resolve))
    }

    @Test
    fun needsDesktopFirstOffersTryAgain() {
        val c = enrollmentContent(EnrollmentStatus.NeedsDesktopFirst, false)
        assertEquals(listOf(EnrollmentAction.TryAgain), c.actions)
        assertEquals("Set up Whimbrel on your Mac first, then come back and sign in here.", resolve(c.bodyRes!!, emptyArray()))
    }

    @Test
    fun removedAndErrorOfferSignIn() {
        val removed = enrollmentContent(EnrollmentStatus.Removed, false)
        assertEquals(listOf(EnrollmentAction.SignIn), removed.actions)
        assertEquals("This phone was removed from your Whimbrel account.", resolve(removed.bodyRes!!, emptyArray()))
        val expired = enrollmentContent(EnrollmentStatus.Error(CoreUserMessage(code = "signin_expired", message = "")), false)
        assertEquals(listOf(EnrollmentAction.SignIn), expired.actions)
        assertEquals("Your Google sign-in expired. Try signing in again.", expired.body!!.localized(resolve))
        assertEquals(listOf(EnrollmentAction.SignIn), enrollmentContent(EnrollmentStatus.Error(null), false).actions)
    }

    @Test
    fun errorShowsItsOwnMessage() {
        val known = enrollmentContent(EnrollmentStatus.Error(CoreUserMessage(code = "network", message = "English")), false)
        assertEquals("Couldn't reach the sync server. Check your connection and try again.", known.body!!.localized(resolve))
        val unknown = enrollmentContent(EnrollmentStatus.Error(CoreUserMessage(code = "brand_new", message = "From the core")), false)
        assertEquals("From the core", unknown.body!!.localized(resolve))
        val none = enrollmentContent(EnrollmentStatus.Error(null), false)
        assertEquals("Something went wrong. Try again.", none.body!!.localized(resolve))
        assertNull(known.bodyRes)
    }

    @Test
    fun signedOutOffersSignIn() {
        val c = enrollmentContent(EnrollmentStatus.SignedOut, false)
        assertEquals(listOf(EnrollmentAction.SignIn), c.actions)
        assertTrue(c.actionsEnabled)
        assertEquals("Sign in with the Google account you use for Whimbrel on your Mac.", resolve(c.bodyRes!!, emptyArray()))
    }

    @Test
    fun notConfiguredShowsItsMessageWithSignInDisabled() {
        val c = enrollmentContent(EnrollmentStatus.NotConfigured("Not set up"), false)
        assertEquals("Not set up", c.body!!.localized(resolve))
        assertEquals(listOf(EnrollmentAction.SignIn), c.actions)
        assertFalse(c.actionsEnabled)
    }

    @Test
    fun deviceLimitOffersUseThisPhoneInsteadPerHolderAndJoinWaitlist() {
        val a = LimitHolder("a", "Pixel")
        val b = LimitHolder("b")
        val c = enrollmentContent(EnrollmentStatus.DeviceLimit("phone", listOf(a, b)), false)
        assertEquals(
            listOf(EnrollmentAction.UseThisPhoneInstead(a), EnrollmentAction.UseThisPhoneInstead(b), EnrollmentAction.JoinWaitlist),
            c.actions,
        )
        assertFalse(EnrollmentAction.SignIn in c.actions)
        assertEquals("Whimbrel's free sync includes one computer and one phone. Pixel is your synced phone.", c.title!!.localized(resolve))
    }
}
