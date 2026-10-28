import Foundation

enum SyncAPIError: Error {
    case httpError(status: Int, body: String)
    case networkError(Error)
    case malformedResponse
}

enum GoogleAuthResult {
    case approved(tursoToken: String, tursoDatabaseUrl: String, wrappedGroupKey: String?)
    case pending
}

struct SyncDeviceRecord: Decodable, Identifiable, Equatable {
    let id: String
    let accountId: String
    let publicKey: String
    let name: String?
    let platform: String?
    let status: String
    let enrolledAtUnixMs: Int64
    let revokedAtUnixMs: Int64?
}

/// Typed client for the sync auth Worker's routes (`POST /v1/auth/google`,
/// `POST /v1/devices`, `POST /v1/devices/:id/approve`, `/revoke`, `/name`). The HTTP
/// calls and response decoding live in core/api::sync_ops::enrollment (M3.0 Task 11);
/// this class runs each blocking bridge call off the main actor and maps the core's
/// error envelope onto `SyncAPIError`, so `SyncState`'s status interpretation is
/// unchanged.
final class SyncClient {
    private let baseURL: URL

    init(baseURL: URL) {
        self.baseURL = baseURL
    }

    /// Non-200 responses throw `SyncAPIError.httpError`; the caller interprets the
    /// status (403 covers both "email not allowed" and "device revoked").
    func authenticateGoogle(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        let base = baseURL.absoluteString
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerAuthenticateGoogle(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey)
        }.value
        return try outcome.get()
    }

    /// Requires the caller's own device to already be `approved` (enforced server-side).
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord] {
        let base = baseURL.absoluteString
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerListDevices(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey)
        }.value
        return try outcome.get()
    }

    func approveDevice(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws {
        let base = baseURL.absoluteString
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerApproveDevice(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey, targetDeviceId: targetDeviceId, wrappedGroupKey: wrappedGroupKey)
        }.value
        try outcome.get()
    }

    /// Self-revocation is allowed, no special-casing.
    func revoke(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws {
        let base = baseURL.absoluteString
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerRevokeDevice(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey, targetDeviceId: targetDeviceId)
        }.value
        try outcome.get()
    }

    /// Always sets the calling device's own name (the route is self-only).
    func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws {
        let base = baseURL.absoluteString
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerSetDeviceName(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey, name: name)
        }.value
        try outcome.get()
    }
}
