package com.mpointer.whimbrel.mobile.core

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * A failed Worker call. [errorJson] is the core's original `{"error":...}` reply, kept so
 * permanence and codes are asked of the core and never re-derived here.
 */
sealed class SyncApiError(val errorJson: String?) : Exception() {
    class Http(val status: Int, val body: String, errorJson: String) : SyncApiError(errorJson) {
        override val message get() = "HTTP $status: $body"
    }

    class Network(errorJson: String) : SyncApiError(errorJson)

    object MalformedResponse : SyncApiError(null)
}

sealed interface GoogleAuthResult {
    data class Approved(val tursoToken: String, val tursoDatabaseUrl: String, val wrappedGroupKey: String?) : GoogleAuthResult
    data object Pending : GoogleAuthResult

    /** `409 device_limit`: the free plan's slot of this [kind] is taken. Nothing was enrolled. */
    data class DeviceLimit(val kind: String, val holders: List<LimitHolder>) : GoogleAuthResult
}

/** A device holding a free-plan slot, as the Worker's `device_limit` lists it. */
@Serializable
data class LimitHolder(val id: String, val name: String? = null, val platform: String? = null)

@Serializable
data class SyncDeviceRecord(
    val id: String,
    val accountId: String,
    val publicKey: String,
    val name: String? = null,
    val platform: String? = null,
    val status: String,
    val enrolledAtUnixMs: Long,
    val revokedAtUnixMs: Long? = null,
)

/** core/api's `GroupKeyDecision`: how to obtain the group key after an approved auth. */
enum class GroupKeyDecision {
    UNWRAP, KEEP, GENERATE, WAIT_FOR_HANDOFF;

    companion object {
        /** [json] is the core's JSON string (`"unwrap"`); anything unparseable waits, never mints a divergent key. */
        fun parse(json: String?): GroupKeyDecision {
            val value = try {
                json?.let { Json.parseToJsonElement(it).jsonPrimitive.contentOrNull }
            } catch (_: Exception) {
                null
            }
            return entries.firstOrNull { it.name.lowercase() == value } ?: WAIT_FOR_HANDOFF
        }

        fun decide(native: CoreNative, wrappedKeyPresent: Boolean, localKeyHeld: Boolean, anotherApprovedDeviceExists: Boolean) =
            parse(native.syncGroupKeyDecision(wrappedKeyPresent, localKeyHeld, anotherApprovedDeviceExists))
    }
}

private val wireJson = Json { ignoreUnknownKeys = true }

/** The core's coded message for a Worker failure (`SyncWorkerClient.userMessage`); null with no error JSON or no code. */
fun coreUserMessage(native: CoreNative, e: SyncApiError): CoreUserMessage? {
    val json = e.errorJson ?: return null
    val coded = native.syncWorkerErrorCode(json) ?: return null
    return try {
        wireJson.decodeFromString(CoreUserMessage.serializer(), coded)
    } catch (_: Exception) {
        null
    }
}

/** Typed client for the sync auth Worker; the HTTP and its envelope live in the core. The phone sends no platform. */
/** `POST /v1/account/plan`; `until` is epoch ms, `interval` "month" or "year", both null off Cloud. */
@Serializable
data class AccountPlan(
    val plan: String,
    val until: Long? = null,
    val interval: String? = null,
    val billing: Boolean = false,
) {
    val isCloud: Boolean get() = plan == "cloud"
}

class SyncWorkerClient(private val native: CoreNative, private val baseUrl: String) {
    suspend fun authenticate(idToken: String, devicePublicKey: String): GoogleAuthResult = call {
        val r = reply(native.syncWorkerAuthenticateGoogle(baseUrl, idToken, devicePublicKey, null))
        when (r.str("status")) {
            "approved" -> GoogleAuthResult.Approved(r.str("turso_token")!!, r.str("turso_database_url")!!, r.str("wrapped_group_key"))
            "pending" -> GoogleAuthResult.Pending
            "deviceLimit" -> GoogleAuthResult.DeviceLimit(
                r.str("kind")!!,
                wireJson.decodeFromJsonElement(ListSerializer(LimitHolder.serializer()), r.getValue("devices")),
            )
            else -> throw SyncApiError.MalformedResponse
        }
    }

    suspend fun listDevices(idToken: String, devicePublicKey: String): List<SyncDeviceRecord> = call {
        val r = reply(native.syncWorkerListDevices(baseUrl, idToken, devicePublicKey))
        wireJson.decodeFromJsonElement(ListSerializer(SyncDeviceRecord.serializer()), r.getValue("devices"))
    }

    suspend fun revoke(idToken: String, devicePublicKey: String, targetDeviceId: String) = call {
        ok(native.syncWorkerRevokeDevice(baseUrl, idToken, devicePublicKey, targetDeviceId))
    }

    suspend fun setDeviceName(idToken: String, devicePublicKey: String, name: String) = call {
        ok(native.syncWorkerSetDeviceName(baseUrl, idToken, devicePublicKey, name))
    }

    suspend fun swap(idToken: String, devicePublicKey: String, replaceDeviceId: String) = call {
        ok(native.syncWorkerSwapDevice(baseUrl, idToken, devicePublicKey, null, replaceDeviceId))
    }

    suspend fun joinWaitlist(idToken: String, devicePublicKey: String, wants: List<String>) = call {
        ok(native.syncWorkerJoinWaitlist(baseUrl, idToken, devicePublicKey, JsonArray(wants.map(::JsonPrimitive)).toString(), null))
    }

    suspend fun accountPlan(idToken: String, devicePublicKey: String): AccountPlan = call {
        wireJson.decodeFromJsonElement(AccountPlan.serializer(), reply(native.syncWorkerAccountPlan(baseUrl, idToken, devicePublicKey)))
    }

    /** Whether polling should stop (401/403), as the core decides from its own reply. */
    fun isPermanent(e: Throwable): Boolean {
        val json = (e as? SyncApiError)?.errorJson ?: return false
        return native.syncWorkerErrorIsPermanent(json)
    }

    fun userMessage(e: SyncApiError): CoreUserMessage? = coreUserMessage(native, e)

    // The FFI call blocks up to 30 s, so it runs on IO. A missing or mistyped field is a malformed reply.
    private suspend fun <T> call(block: () -> T): T = withContext(Dispatchers.IO) {
        try {
            block()
        } catch (e: SyncApiError) {
            throw e
        } catch (_: Exception) {
            throw SyncApiError.MalformedResponse
        }
    }

    /** The reply object, or the [SyncApiError] its `error` envelope (or its absence) stands for. */
    private fun reply(raw: String?): JsonObject {
        if (raw == null) throw SyncApiError.MalformedResponse
        val root = wireJson.parseToJsonElement(raw).jsonObject
        val error = root["error"]?.jsonObject ?: return root
        return when (error.str("kind")) {
            "http" -> throw SyncApiError.Http(error.getValue("status").jsonPrimitive.int, error.str("body") ?: "", raw)
            "network" -> throw SyncApiError.Network(raw)
            else -> throw SyncApiError.MalformedResponse
        }
    }

    private fun ok(raw: String?) {
        if (reply(raw)["ok"]?.jsonPrimitive?.contentOrNull != "true") throw SyncApiError.MalformedResponse
    }

    private fun JsonObject.str(key: String): String? = this[key]?.jsonPrimitive?.contentOrNull
}
