import Foundation

enum SyncAPIError: Error {
    case httpError(status: Int, body: String)
    case networkError(Error)
    case malformedResponse
}

enum GoogleAuthResult {
    case approved(tursoToken: String, tursoDatabaseUrl: String, wrappedGroupKey: String?)
    case pending
    /// `409 device_limit`: the free plan's computer or phone slot is taken. Nothing was enrolled.
    case deviceLimit(kind: String, holders: [LimitHolder])
}

/// A device holding a free-plan slot, as the Worker's `device_limit` lists it.
struct LimitHolder: Decodable, Equatable {
    let id: String
    let name: String?
    let platform: String?
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

/// The Worker calls `SyncState` makes: `SyncClient` in the app, a fake in tests.
protocol SyncWorker: AnyObject {
    func authenticateGoogle(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord]
    func approveDevice(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws
    func revoke(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws
    func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws
    func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws
    func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws
    func resetAccount(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult
    func deleteAccount(idToken: String, devicePublicKey: String) async throws
}

/// Typed client for the sync auth Worker's routes (`POST /v1/auth/google`,
/// `POST /v1/devices`, `POST /v1/devices/:id/approve`, `/revoke`, `/name`, `/swap`,
/// `/v1/waitlist`, `/v1/account/reset`, `/v1/account/delete`). The HTTP
/// calls and response decoding live in core/api::sync_ops::enrollment (M3.0 Task 11);
/// this class runs each blocking bridge call off the main actor and maps the core's
/// error envelope onto `SyncAPIError`, so `SyncState`'s status interpretation is
/// unchanged.
final class SyncClient: SyncWorker {
    private let baseURL: URL
    /// "mac" or "windows". The iPhone leaves it nil: the Worker infers `ios` from the
    /// Google audience.
    private let platform: String?

    init(baseURL: URL, platform: String? = nil) {
        self.baseURL = baseURL
        self.platform = platform
    }

    /// Non-200 responses throw `SyncAPIError.httpError`; the caller interprets the
    /// status (403 covers both "email not allowed" and "device revoked").
    func authenticateGoogle(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        let base = baseURL.absoluteString
        let platform = platform
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerAuthenticateGoogle(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey, platform: platform)
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

    /// "Use this computer instead": this device becomes a pending swap for `replaceDeviceId`.
    func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws {
        let base = baseURL.absoluteString
        let platform = platform
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerSwapDevice(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey, platform: platform, replaceDeviceId: replaceDeviceId)
        }.value
        try outcome.get()
    }

    func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws {
        let base = baseURL.absoluteString
        let platform = platform
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerJoinWaitlist(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey, wants: wants, platform: platform)
        }.value
        try outcome.get()
    }

    /// The fresh start. Success is `.approved` with no wrapped key.
    func resetAccount(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        let base = baseURL.absoluteString
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerResetAccount(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey)
        }.value
        return try outcome.get()
    }

    func deleteAccount(idToken: String, devicePublicKey: String) async throws {
        let base = baseURL.absoluteString
        let outcome = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.syncWorkerDeleteAccount(baseURL: base, idToken: idToken, devicePublicKey: devicePublicKey)
        }.value
        try outcome.get()
    }
}
