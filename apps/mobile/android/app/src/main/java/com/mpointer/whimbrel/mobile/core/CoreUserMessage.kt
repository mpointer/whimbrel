package com.mpointer.whimbrel.mobile.core

import com.mpointer.whimbrel.mobile.R
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/**
 * A core message as `{"code","params","message"}`: the stable code and params the app renders from
 * strings.xml, beside the core's English [message]. An unknown code, or a known one missing a param,
 * shows [message]. Mirrors `CoreUserMessage.swift`.
 */
@Serializable
data class CoreUserMessage(val code: String? = null, val params: Map<String, JsonElement>? = null, val message: String) {
    /** [context] resolves a string resource id and its format args, e.g. `{ id, args -> ctx.getString(id, *args) }`. */
    fun localized(context: (Int, Array<Any>) -> String): String {
        fun text(id: Int, vararg args: Any) = context(id, arrayOf(*args))
        fun param(key: String) = (params?.get(key) as? JsonPrimitive)?.contentOrNull
        return when (code) {
            "device_revoked" -> text(R.string.core_msg_device_revoked)
            "device_not_approved" -> text(R.string.core_msg_device_not_approved)
            "approve_on_other_device_unnamed" -> text(R.string.core_msg_approve_on_other_device_unnamed)
            "approve_on_other_device" -> {
                val names = (params?.get("devices") as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
                if (names.isNullOrEmpty()) message else text(R.string.core_msg_approve_on_other_device, orList(names))
            }
            "reset_requires_computer" -> text(R.string.core_msg_reset_requires_computer)
            "account_not_allowed" -> text(R.string.core_msg_account_not_allowed)
            "signin_expired" -> text(R.string.core_msg_signin_expired)
            "google_signin_failed" -> text(R.string.core_msg_google_signin_failed)
            "device_limit" -> text(R.string.core_msg_device_limit)
            "rate_limited" -> text(R.string.core_msg_rate_limited)
            "account_changing" -> text(R.string.core_msg_account_changing)
            "swap_kind_mismatch" -> text(R.string.core_msg_swap_kind_mismatch)
            "wants_required" -> text(R.string.core_msg_wants_required)
            "reset_confirm_required" -> text(R.string.core_msg_reset_confirm_required)
            "device_not_found" -> text(R.string.core_msg_device_not_found)
            "server_error" -> param("status")?.let { text(R.string.core_msg_server_error, it) } ?: message
            "network" -> text(R.string.core_msg_network)
            "malformed_response" -> text(R.string.core_msg_malformed_response)
            else -> message
        }
    }

    // English "or" list: "A", "A or B", "A, B, or C" (what Swift's .list(type: .or) gives).
    private fun orList(names: List<String>) = when (names.size) {
        1 -> names[0]
        2 -> "${names[0]} or ${names[1]}"
        else -> names.dropLast(1).joinToString(", ") + ", or " + names.last()
    }
}
