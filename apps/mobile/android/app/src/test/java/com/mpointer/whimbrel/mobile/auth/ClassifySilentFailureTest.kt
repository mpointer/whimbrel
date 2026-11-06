package com.mpointer.whimbrel.mobile.auth

import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialInterruptedException
import androidx.credentials.exceptions.GetCredentialProviderConfigurationException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import androidx.credentials.exceptions.NoCredentialException
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class ClassifySilentFailureTest {
    @Test fun needsInteractiveSignInIsNotSignedIn() {
        listOf<GetCredentialException>(
            NoCredentialException(), GetCredentialCancellationException(), GetCredentialUnsupportedException(),
        ).forEach { assertSame(it.javaClass.simpleName, GoogleSignInError.NotSignedIn, classifySilentFailure(it, online = true)) }
    }

    @Test fun networkOrPlayServicesIsTransient() {
        listOf<GetCredentialException>(
            GetCredentialInterruptedException(), GetCredentialProviderConfigurationException(),
            // One Tap's "7: Network error" arrives as Unknown.
            GetCredentialUnknownException(),
        ).forEach { assertTrue(it.javaClass.simpleName, classifySilentFailure(it, online = true) is GoogleSignInError.Failed) }
    }

    @Test fun noCredentialWhileOfflineIsTransient() {
        assertTrue(classifySilentFailure(NoCredentialException(), online = false) is GoogleSignInError.Failed)
    }

    @Test fun userCancelStaysSilent() {
        assertSame(GoogleSignInError.Cancelled, classifyInteractiveFailure(GetCredentialCancellationException()))
        assertSame(GoogleSignInError.Cancelled, classifyInteractiveFailure(GetCredentialCancellationException("activity is cancelled by the user.")))
    }

    @Test fun providerErrorsAreFailures() {
        listOf<GetCredentialException>(
            NoCredentialException(), GetCredentialUnknownException(), GetCredentialProviderConfigurationException(),
            GetCredentialInterruptedException(),
            // Google reports a misconfigured OAuth client as a "cancellation".
            GetCredentialCancellationException("UNREGISTERED_ON_API_CONSOLE"),
        ).forEach { assertTrue(it.javaClass.simpleName, classifyInteractiveFailure(it) is GoogleSignInError.Failed) }
    }
}
