package com.mpointer.whimbrel.mobile.core

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class SyncWireTest {
    private fun client(reply: String?) = FakeSyncNative(reply).let { it to SyncWorkerClient(it, "https://w.example") }

    private fun auth(reply: String?) = runBlocking { client(reply).second.authenticate("tok", "pk") }

    private fun failure(reply: String?): SyncApiError =
        try {
            auth(reply)
            fail("expected SyncApiError")
            error("unreachable")
        } catch (e: SyncApiError) {
            e
        }

    @Test
    fun approvedReplyDecodesWithNullKey() {
        val r = auth("""{"status":"approved","turso_token":"t","turso_database_url":"u","wrapped_group_key":null}""")
        assertEquals(GoogleAuthResult.Approved("t", "u", null), r)
    }

    @Test
    fun approvedReplyKeepsWrappedKey() {
        val r = auth("""{"status":"approved","turso_token":"t","turso_database_url":"u","wrapped_group_key":"w"}""")
        assertEquals(GoogleAuthResult.Approved("t", "u", "w"), r)
    }

    @Test
    fun pendingReply() = assertEquals(GoogleAuthResult.Pending, auth("""{"status":"pending"}"""))

    @Test
    fun deviceLimitReply() {
        val r = auth("""{"status":"deviceLimit","kind":"phone","devices":[{"id":"d","name":null,"platform":"ios"}]}""")
        assertEquals(GoogleAuthResult.DeviceLimit("phone", listOf(LimitHolder("d", null, "ios"))), r)
    }

    @Test
    fun httpErrorKeepsTheCoresJson() {
        val json = """{"error":{"kind":"http","status":403,"body":"device revoked"}}"""
        val e = failure(json) as SyncApiError.Http
        assertEquals(403, e.status)
        assertEquals("device revoked", e.body)
        assertEquals(json, e.errorJson)
    }

    @Test
    fun networkError() {
        val json = """{"error":{"kind":"network"}}"""
        val e = failure(json) as SyncApiError.Network
        assertEquals(json, e.errorJson)
    }

    @Test
    fun nullReturnIsMalformed() = assertTrue(failure(null) === SyncApiError.MalformedResponse)

    @Test
    fun coreMalformedEnvelopeAndUnknownStatusAreMalformed() {
        assertTrue(failure("""{"error":{"kind":"malformed_response"}}""") === SyncApiError.MalformedResponse)
        assertTrue(failure("""{"status":"surprise"}""") === SyncApiError.MalformedResponse)
        assertTrue(failure("not json") === SyncApiError.MalformedResponse)
    }

    @Test
    fun isPermanentAsksTheCoreWithTheOriginalJson() {
        val json = """{"error":{"kind":"http","status":401,"body":"x"}}"""
        val (fake, c) = client(json)
        fake.permanentReply = true
        val e = failure(json)
        assertTrue(c.isPermanent(e))
        assertEquals(json, fake.permanentArg)
    }

    @Test
    fun isPermanentIsFalseWithoutAnErrorJson() {
        val (fake, c) = client(null)
        fake.permanentReply = true
        assertFalse(c.isPermanent(SyncApiError.MalformedResponse))
        assertFalse(c.isPermanent(IllegalStateException()))
        assertNull(fake.permanentArg)
    }

    @Test
    fun userMessageDecodesTheCoresCode() {
        val json = """{"error":{"kind":"http","status":403,"body":"x"}}"""
        val (fake, c) = client(json)
        fake.errorCodeReply = """{"code":"device_revoked","params":null,"message":"revoked"}"""
        val m = c.userMessage(failure(json))
        assertEquals(CoreUserMessage("device_revoked", null, "revoked"), m)
        assertEquals(json, fake.errorCodeArg)
        assertNull(c.userMessage(SyncApiError.MalformedResponse))
    }

    @Test
    fun listDevicesDecodes() {
        val (_, c) = client(
            """{"devices":[{"id":"d","accountId":"a","publicKey":"k","name":null,"platform":"mac","status":"approved","enrolledAtUnixMs":5,"revokedAtUnixMs":null}]}""",
        )
        val d = runBlocking { c.listDevices("tok", "pk") }
        assertEquals(listOf(SyncDeviceRecord("d", "a", "k", null, "mac", "approved", 5L, null)), d)
    }

    @Test
    fun okRepliesPassAndErrorsThrow() {
        val (_, c) = client("""{"ok":true}""")
        runBlocking {
            c.revoke("t", "pk", "d")
            c.setDeviceName("t", "pk", "Pixel")
            c.swap("t", "pk", "d")
            c.joinWaitlist("t", "pk", listOf("a", "b"))
        }
        val (_, bad) = client("""{"error":{"kind":"network"}}""")
        try {
            runBlocking { bad.revoke("t", "pk", "d") }
            fail("expected Network")
        } catch (_: SyncApiError.Network) {
        }
    }

    @Test
    fun phoneSendsNoPlatformAndWantsGoAsJson() {
        val (fake, c) = client("""{"status":"pending"}""")
        runBlocking { c.authenticate("t", "pk") }
        assertNull(fake.lastPlatform)
        fake.lastPlatform = "unset"
        fake.reply = """{"ok":true}"""
        runBlocking { c.swap("t", "pk", "d") }
        assertNull(fake.lastPlatform)
        fake.lastPlatform = "unset"
        runBlocking { c.joinWaitlist("t", "pk", listOf("a", "b")) }
        assertNull(fake.lastPlatform)
        assertEquals("""["a","b"]""", fake.lastWants)
    }

    @Test
    fun groupKeyDecisionParsesAllValuesAndMapsGarbageToWait() {
        assertEquals(GroupKeyDecision.UNWRAP, GroupKeyDecision.parse("\"unwrap\""))
        assertEquals(GroupKeyDecision.KEEP, GroupKeyDecision.parse("\"keep\""))
        assertEquals(GroupKeyDecision.GENERATE, GroupKeyDecision.parse("\"generate\""))
        assertEquals(GroupKeyDecision.WAIT_FOR_HANDOFF, GroupKeyDecision.parse("\"wait_for_handoff\""))
        assertEquals(GroupKeyDecision.WAIT_FOR_HANDOFF, GroupKeyDecision.parse("\"bogus\""))
        assertEquals(GroupKeyDecision.WAIT_FOR_HANDOFF, GroupKeyDecision.parse("garbage"))
        assertEquals(GroupKeyDecision.WAIT_FOR_HANDOFF, GroupKeyDecision.parse(null))
    }

    @Test
    fun accountPlanParsesCloudAndFree() {
        val (fake, c) = client("""{"plan":"cloud","until":1790000000000,"interval":"year","renews":true,"billing":true}""")
        val cloud = runBlocking { c.accountPlan("t", "pk") }
        assertEquals(AccountPlan("cloud", 1790000000000L, "year", true), cloud)
        assertTrue(cloud.isCloud)
        assertEquals(listOf("https://w.example", "t", "pk"), fake.planAccountArgs)
        fake.reply = """{"plan":"free","until":null,"interval":null,"billing":false}"""
        val free = runBlocking { c.accountPlan("t", "pk") }
        assertEquals(AccountPlan("free"), free)
        assertFalse(free.isCloud)
        fake.reply = """{"error":{"kind":"network"}}"""
        try {
            runBlocking { c.accountPlan("t", "pk") }
            fail("expected Network")
        } catch (_: SyncApiError.Network) {
        }
    }
}
