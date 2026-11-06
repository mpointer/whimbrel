package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord

/** What to call a device. The model has no Context, so the fallbacks are resolved by the UI with [localized]. */
sealed interface DeviceLabel {
    data class Named(val name: String) : DeviceLabel
    data object ThisPhone : DeviceLabel
    data class ShortId(val prefix: String) : DeviceLabel

    /** [context] resolves a string resource id and its format args, as for `CoreUserMessage.localized`. */
    fun localized(context: (Int, Array<Any>) -> String): String = when (this) {
        is Named -> name
        ThisPhone -> context(R.string.device_label_this_phone, emptyArray())
        is ShortId -> context(R.string.device_label_short_id, arrayOf(prefix))
    }
}

/**
 * Names for the account's devices, so a meeting says where it was recorded: the server's device list
 * (public key to name), from `DeviceLabels.swift`. Meetings carry the public key.
 */
data class DeviceLabels(val names: Map<String, String> = emptyMap(), val ownDeviceId: String? = null) {
    fun label(deviceId: String): DeviceLabel = when {
        names[deviceId] != null -> DeviceLabel.Named(names.getValue(deviceId))
        deviceId == ownDeviceId -> DeviceLabel.ThisPhone
        else -> DeviceLabel.ShortId(deviceId.take(6))
    }

    companion object {
        /** Trimmed, non-empty names; the first record for a key wins. */
        fun names(records: List<SyncDeviceRecord>): Map<String, String> {
            val result = mutableMapOf<String, String>()
            for (record in records) {
                val name = record.name?.trim().orEmpty()
                if (name.isNotEmpty()) result.putIfAbsent(record.publicKey, name)
            }
            return result
        }
    }
}
