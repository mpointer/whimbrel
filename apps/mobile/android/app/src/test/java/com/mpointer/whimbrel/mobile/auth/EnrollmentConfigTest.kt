package com.mpointer.whimbrel.mobile.auth

import com.mpointer.whimbrel.mobile.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EnrollmentConfigTest {
    private val goodId = "123-abc.apps.googleusercontent.com"
    private val goodUrl = "https://sync.example.com"

    @Test fun clientIdRule() {
        assertFalse(EnrollmentConfig.isUsableClientId(""))
        assertFalse(EnrollmentConfig.isUsableClientId("REPLACE_ME"))
        assertFalse(EnrollmentConfig.isUsableClientId(".apps.googleusercontent.com"))
        assertTrue(EnrollmentConfig.isUsableClientId(goodId))
    }

    @Test fun workerUrlRule() {
        assertFalse(EnrollmentConfig.isUsableWorkerUrl(""))
        assertFalse(EnrollmentConfig.isUsableWorkerUrl("http://sync.example.com"))
        assertFalse(EnrollmentConfig.isUsableWorkerUrl("https://"))
        assertTrue(EnrollmentConfig.isUsableWorkerUrl(goodUrl))
    }

    @Test fun configErrorResIsNullOnlyWhenBothAreUsable() {
        assertNull(EnrollmentConfig.configErrorRes(goodId, goodUrl))
        assertEquals(R.string.config_missing_client_id, EnrollmentConfig.configErrorRes("", goodUrl))
        assertEquals(R.string.config_missing_worker_url, EnrollmentConfig.configErrorRes(goodId, ""))
        assertEquals(R.string.config_missing_both, EnrollmentConfig.configErrorRes("", ""))
    }
}
