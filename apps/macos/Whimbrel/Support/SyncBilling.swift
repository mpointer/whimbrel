import Foundation

/// `POST /v1/account/plan`. `until` (ms) and `interval` ("month" | "year") are nil off Cloud;
/// `billing` is false when the server has no Stripe settings, and then nothing offers to pay.
struct AccountPlan: Decodable, Equatable {
    let plan: String
    let until: Int64?
    let interval: String?
    let billing: Bool

    var isCloud: Bool { plan == "cloud" }
}

/// The billing routes `SyncState` calls: `SyncBillingClient` in the app, a fake in tests.
/// Mac-only, so `SyncWorker` (shared with the iPhone) stays as it is.
protocol SyncBilling: AnyObject {
    func accountPlan(idToken: String, devicePublicKey: String) async throws -> AccountPlan
    func checkoutURL(idToken: String, devicePublicKey: String, interval: String) async throws -> URL
    func portalURL(idToken: String, devicePublicKey: String) async throws -> URL
}

/// The core's blocking billing calls, run off the main actor, with its error envelope mapped
/// onto `SyncAPIError` the way `SyncClient` does.
final class SyncBillingClient: SyncBilling {
    private let baseURL: URL

    init(baseURL: URL) {
        self.baseURL = baseURL
    }

    func accountPlan(idToken: String, devicePublicKey: String) async throws -> AccountPlan {
        let base = baseURL.absoluteString
        return try await Task.detached(priority: .userInitiated) {
            let json = base.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in
                whimbrel_sync_worker_account_plan(b, t, k)
            } } }
            return Self.outcome(json, AccountPlan.self)
        }.value.get()
    }

    func checkoutURL(idToken: String, devicePublicKey: String, interval: String) async throws -> URL {
        let base = baseURL.absoluteString
        return try await Task.detached(priority: .userInitiated) {
            let json = base.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in interval.withCString { i in
                whimbrel_sync_worker_billing_checkout(b, t, k, i)
            } } } }
            return Self.outcome(json, URLReply.self).flatMap(\.parsed)
        }.value.get()
    }

    func portalURL(idToken: String, devicePublicKey: String) async throws -> URL {
        let base = baseURL.absoluteString
        return try await Task.detached(priority: .userInitiated) {
            let json = base.withCString { b in idToken.withCString { t in devicePublicKey.withCString { k in
                whimbrel_sync_worker_billing_portal(b, t, k)
            } } }
            return Self.outcome(json, URLReply.self).flatMap(\.parsed)
        }.value.get()
    }

    private struct URLReply: Decodable {
        let url: String
        /// Only an https URL goes to the browser.
        var parsed: Result<URL, SyncAPIError> {
            guard let url = URL(string: url), url.scheme == "https" else { return .failure(.malformedResponse) }
            return .success(url)
        }
    }

    private struct ErrorEnvelope: Decodable {
        struct Inner: Decodable { let kind: String; let status: Int?; let body: String? }
        let error: Inner?
    }

    /// The success payload, or the core's `{"error": {...}}` envelope as `SyncAPIError`.
    /// A copy of `CoreMemoryStore.syncWorkerOutcome` and `SyncWorkerErrorEnvelope.apiError` in
    /// SyncCoreBridge.swift, which are private to that file (shared with iOS, not changed here);
    /// keep the two mappings the same.
    private static func outcome<T: Decodable>(_ json: UnsafeMutablePointer<CChar>?, _ type: T.Type) -> Result<T, SyncAPIError> {
        guard let json else { return .failure(.malformedResponse) }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        if let inner = (try? JSONDecoder().decode(ErrorEnvelope.self, from: data))?.error {
            switch inner.kind {
            case "http": return .failure(.httpError(status: inner.status ?? -1, body: inner.body ?? ""))
            case "network": return .failure(.networkError(URLError(.cannotConnectToHost)))
            default: return .failure(.malformedResponse)
            }
        }
        guard let value = try? JSONDecoder().decode(T.self, from: data) else { return .failure(.malformedResponse) }
        return .success(value)
    }
}

enum PlanCopy {
    /// "Free: 1 computer + 1 phone" or "Cloud until November 9, 2027" (in the user's locale).
    static func line(_ plan: AccountPlan) -> String {
        guard plan.isCloud, let until = plan.until else { return String(localized: "Free: 1 computer + 1 phone") }
        let date = Date(timeIntervalSince1970: Double(until) / 1000).formatted(.dateTime.month(.wide).day().year())
        return String(localized: "Cloud until \(date)")
    }
}
