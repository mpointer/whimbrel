package com.mpointer.whimbrel.mobile.core

import com.mpointer.whimbrel.mobile.sync.SyncCredentials
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.Json

/** A settings field as the core returns it (`pulled`, keyed by field, so [fieldKey] is absent) or plans it (`fields`). */
@Serializable
data class PulledSettingsField(
    @SerialName("field_key") val fieldKey: String = "",
    val value: String,
    @SerialName("updated_at_unix_ms") val updatedAtUnixMs: Long,
    @SerialName("is_deleted") val isDeleted: Boolean,
)

interface SettingsPulling {
    /** One blocking settings round trip (never on the main thread); null on any failure. */
    fun iterate(c: SyncCredentials, localJson: String): Map<String, PulledSettingsField>?
    fun applyPlan(pulled: Map<String, PulledSettingsField>, localTimestamps: Map<String, Long>, apiKeySyncEnabled: Map<String, Boolean>): List<PulledSettingsField>?
}

class CoreSettingsPuller(private val native: CoreNative) : SettingsPulling {
    private val json = Json { ignoreUnknownKeys = true }
    private val fields = MapSerializer(String.serializer(), PulledSettingsField.serializer())

    @Serializable private class Iteration(val pulled: Map<String, PulledSettingsField>)
    @Serializable private class Plan(val fields: List<PulledSettingsField>)

    override fun iterate(c: SyncCredentials, localJson: String): Map<String, PulledSettingsField>? =
        native.syncSettingsIteration(c.tursoDatabaseUrl, c.tursoToken, c.groupKeyBase64, localJson)
            ?.let { runCatching { json.decodeFromString<Iteration>(it).pulled }.getOrNull() }

    override fun applyPlan(
        pulled: Map<String, PulledSettingsField>,
        localTimestamps: Map<String, Long>,
        apiKeySyncEnabled: Map<String, Boolean>,
    ): List<PulledSettingsField>? =
        native.syncSettingsApplyPlan(
            json.encodeToString(fields, pulled),
            json.encodeToString(MapSerializer(String.serializer(), Long.serializer()), localTimestamps),
            json.encodeToString(MapSerializer(String.serializer(), Boolean.serializer()), apiKeySyncEnabled),
        )?.let { runCatching { json.decodeFromString<Plan>(it).fields }.getOrNull() }
}
