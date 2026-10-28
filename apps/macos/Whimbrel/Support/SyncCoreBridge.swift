import Foundation

// Sync-enrollment wrappers over core/ffi, moved verbatim out of CoreBridge.swift so the
// iOS app compiles them too (CoreBridge.swift itself is Mac-only). The iOS app declares
// the `CoreMemoryStore` type itself (apps/mobile/ios/Whimbrel/CoreMemoryStore+iOS.swift).

/// JSON shape matching core/sync's `DeviceKeypair` — a device's long-term X25519
/// keypair for multi-device enrollment and key-wrapping (FR: device enrollment).
struct CoreSyncDeviceKeypair: Decodable {
    let publicKeyBase64: String
    let secretKeyBase64: String

    enum CodingKeys: String, CodingKey {
        case publicKeyBase64 = "public_key_base64"
        case secretKeyBase64 = "secret_key_base64"
    }
}

/// core/api::sync_ops::enrollment's `GroupKeyDecision`.
enum SyncGroupKeyDecision: String, Decodable {
    case unwrap
    case keep
    case generate
    case waitForHandoff = "wait_for_handoff"
}

extension CoreMemoryStore {
    /// Generates a fresh device keypair. Persist `secretKeyBase64` in the
    /// Keychain; send `publicKeyBase64` to the server as the device's
    /// devicePublicKey. Returns nil only on an unexpected internal failure.
    static func generateSyncDeviceKeypair() -> CoreSyncDeviceKeypair? {
        guard let json = whimbrel_sync_generate_device_keypair() else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        return try? JSONDecoder().decode(CoreSyncDeviceKeypair.self, from: data)
    }

    /// Re-derives a device's public key from its persisted secret key bytes.
    /// Returns nil on a malformed `secretKeyBase64`.
    static func syncDevicePublicKey(secretKeyBase64: String) -> String? {
        guard let json = secretKeyBase64.withCString({ whimbrel_sync_device_public_key($0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Response: Decodable {
            let publicKeyBase64: String
            enum CodingKeys: String, CodingKey { case publicKeyBase64 = "public_key_base64" }
        }
        let data = Data(String(cString: json).utf8)
        let response: Response? = try? JSONDecoder().decode(Response.self, from: data)
        return response?.publicKeyBase64
    }

    /// Short code derived from a device's base64 public key (e.g. "CC · 6W · TA · B6 · RG"),
    /// shown on both devices when approving. Nil for a malformed or wrong-length key.
    static func deviceComparisonCode(publicKeyBase64: String) -> String? {
        guard let ptr = publicKeyBase64.withCString({ whimbrel_device_comparison_code($0) }) else { return nil }
        defer { whimbrel_core_free_string(ptr) }
        return String(cString: ptr)
    }

    /// Generates a fresh group key (the first device on an account calls this
    /// once; later devices receive the same key via wrap/unwrap instead).
    /// Returns the key's base64 encoding, or nil on an unexpected internal
    /// failure.
    static func generateSyncGroupKey() -> String? {
        guard let json = whimbrel_sync_generate_group_key() else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Response: Decodable {
            let groupKeyBase64: String
            enum CodingKeys: String, CodingKey { case groupKeyBase64 = "group_key_base64" }
        }
        let data = Data(String(cString: json).utf8)
        let response: Response? = try? JSONDecoder().decode(Response.self, from: data)
        return response?.groupKeyBase64
    }

    /// Wraps a group key for a specific recipient device's public key. Used by
    /// an already-approved device when approving a pending device: its own
    /// secret key is `senderSecretKeyBase64`, the pending device's public key
    /// (from `POST /v1/devices`) is `recipientPublicKeyBase64`, and its own
    /// held group key is `groupKeyBase64`. The result is sent as-is to
    /// `POST /v1/devices/:id/approve`'s `wrappedGroupKey` field. Returns nil on
    /// any malformed input or a low-order recipient public key.
    static func wrapSyncGroupKey(senderSecretKeyBase64: String, recipientPublicKeyBase64: String, groupKeyBase64: String) -> String? {
        guard let json = senderSecretKeyBase64.withCString({ sender in
            recipientPublicKeyBase64.withCString { recipient in
                groupKeyBase64.withCString { key in
                    whimbrel_sync_wrap_group_key(sender, recipient, key)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Response: Decodable {
            let wrappedGroupKeyBase64: String
            enum CodingKeys: String, CodingKey { case wrappedGroupKeyBase64 = "wrapped_group_key_base64" }
        }
        let data = Data(String(cString: json).utf8)
        let response: Response? = try? JSONDecoder().decode(Response.self, from: data)
        return response?.wrappedGroupKeyBase64
    }

    /// Recovers a group key from a wrapped blob (the `wrappedGroupKey` field
    /// from `POST /v1/auth/google`'s response). `recipientSecretKeyBase64` is
    /// this device's own persisted secret key; `senderPublicKeyBase64` must
    /// come from an already-trusted source, never from data embedded in the
    /// wrapped blob. Returns nil on any malformed input or an unwrap failure.
    static func unwrapSyncGroupKey(recipientSecretKeyBase64: String, senderPublicKeyBase64: String, wrappedGroupKeyBase64: String) -> String? {
        guard let json = recipientSecretKeyBase64.withCString({ recipient in
            senderPublicKeyBase64.withCString { sender in
                wrappedGroupKeyBase64.withCString { wrapped in
                    whimbrel_sync_unwrap_group_key(recipient, sender, wrapped)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Response: Decodable {
            let groupKeyBase64: String
            enum CodingKeys: String, CodingKey { case groupKeyBase64 = "group_key_base64" }
        }
        let data = Data(String(cString: json).utf8)
        let response: Response? = try? JSONDecoder().decode(Response.self, from: data)
        return response?.groupKeyBase64
    }

    // MARK: - Sync enrollment (core/api::sync_ops::enrollment, M3.0 Task 11)

    /// The core's `{"error": {...}}` envelope mapped onto `SyncAPIError`.
    private struct SyncWorkerErrorEnvelope: Decodable {
        struct Inner: Decodable { let kind: String; let status: Int?; let body: String? }
        let error: Inner?

        var apiError: SyncAPIError {
            switch error?.kind {
            case "http": return .httpError(status: error?.status ?? -1, body: error?.body ?? "")
            case "network": return .networkError(URLError(.cannotConnectToHost))
            default: return .malformedResponse
            }
        }
    }

    /// Decodes a Worker call's result: the success payload, or the error envelope.
    private static func syncWorkerOutcome<T: Decodable>(_ json: UnsafeMutablePointer<CChar>?, _ type: T.Type) -> Result<T, SyncAPIError> {
        guard let json else { return .failure(.malformedResponse) }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        if let envelope = try? JSONDecoder().decode(SyncWorkerErrorEnvelope.self, from: data), envelope.error != nil {
            return .failure(envelope.apiError)
        }
        guard let value = try? JSONDecoder().decode(T.self, from: data) else { return .failure(.malformedResponse) }
        return .success(value)
    }

    private struct SyncAuthReply: Decodable {
        let status: String
        let tursoToken: String?
        let tursoDatabaseUrl: String?
        let wrappedGroupKey: String?

        enum CodingKeys: String, CodingKey {
            case status
            case tursoToken = "turso_token"
            case tursoDatabaseUrl = "turso_database_url"
            case wrappedGroupKey = "wrapped_group_key"
        }
    }

    /// BLOCKING: `POST /v1/auth/google`. Call off the main actor.
    static func syncWorkerAuthenticateGoogle(baseURL: String, idToken: String, devicePublicKey: String) -> Result<GoogleAuthResult, SyncAPIError> {
        let json = baseURL.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in whimbrel_sync_worker_authenticate_google(b, t, k) } } }
        return syncWorkerOutcome(json, SyncAuthReply.self).flatMap { reply in
            switch reply.status {
            case "pending": return .success(.pending)
            case "approved":
                guard let token = reply.tursoToken, let url = reply.tursoDatabaseUrl else { return .failure(.malformedResponse) }
                return .success(.approved(tursoToken: token, tursoDatabaseUrl: url, wrappedGroupKey: reply.wrappedGroupKey))
            default: return .failure(.malformedResponse)
            }
        }
    }

    /// BLOCKING: `POST /v1/devices`.
    static func syncWorkerListDevices(baseURL: String, idToken: String, devicePublicKey: String) -> Result<[SyncDeviceRecord], SyncAPIError> {
        struct Reply: Decodable { let devices: [SyncDeviceRecord] }
        let json = baseURL.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in whimbrel_sync_worker_list_devices(b, t, k) } } }
        return syncWorkerOutcome(json, Reply.self).map(\.devices)
    }

    private struct OkReply: Decodable { let ok: Bool }

    /// BLOCKING: `POST /v1/devices/:id/approve`.
    static func syncWorkerApproveDevice(baseURL: String, idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) -> Result<Void, SyncAPIError> {
        let json = baseURL.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in targetDeviceId.withCString { d in wrappedGroupKey.withCString { w in
            whimbrel_sync_worker_approve_device(b, t, k, d, w)
        } } } } }
        return syncWorkerOutcome(json, OkReply.self).map { _ in () }
    }

    /// BLOCKING: `POST /v1/devices/:id/revoke`.
    static func syncWorkerRevokeDevice(baseURL: String, idToken: String, devicePublicKey: String, targetDeviceId: String) -> Result<Void, SyncAPIError> {
        let json = baseURL.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in targetDeviceId.withCString { d in
            whimbrel_sync_worker_revoke_device(b, t, k, d)
        } } } }
        return syncWorkerOutcome(json, OkReply.self).map { _ in () }
    }

    /// BLOCKING: `POST /v1/devices/name`.
    static func syncWorkerSetDeviceName(baseURL: String, idToken: String, devicePublicKey: String, name: String) -> Result<Void, SyncAPIError> {
        let json = baseURL.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in name.withCString { n in
            whimbrel_sync_worker_set_device_name(b, t, k, n)
        } } } }
        return syncWorkerOutcome(json, OkReply.self).map { _ in () }
    }

    /// How to obtain the group key after an "approved" auth (core/api's decision table).
    static func syncGroupKeyDecision(wrappedKeyPresent: Bool, localKeyHeld: Bool, anotherApprovedDeviceExists: Bool) -> SyncGroupKeyDecision {
        guard let json = whimbrel_sync_group_key_decision(wrappedKeyPresent, localKeyHeld, anotherApprovedDeviceExists) else { return .waitForHandoff }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONDecoder().decode(SyncGroupKeyDecision.self, from: Data(String(cString: json).utf8))) ?? .waitForHandoff
    }

    /// Whether a Worker failure is permanent for this device/account (401/403), and
    /// its user-facing message, from core/api's tables.
    static func syncWorkerErrorIsPermanent(_ error: SyncAPIError) -> Bool {
        syncWorkerErrorJSON(error).withCString { whimbrel_sync_worker_error_is_permanent($0) }
    }

    static func syncWorkerErrorMessage(_ error: SyncAPIError) -> String? {
        guard let json = syncWorkerErrorJSON(error).withCString({ whimbrel_sync_worker_error_message($0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return String(cString: json)
    }

    private static func syncWorkerErrorJSON(_ error: SyncAPIError) -> String {
        let object: [String: Any]
        switch error {
        case let .httpError(status, body): object = ["kind": "http", "status": status, "body": body]
        case .networkError: object = ["kind": "network"]
        case .malformedResponse: object = ["kind": "malformed_response"]
        }
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }
}
