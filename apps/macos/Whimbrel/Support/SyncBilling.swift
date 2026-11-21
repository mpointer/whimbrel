import Foundation

/// `POST /v1/account/plan`. `until` (ms) and `interval` ("month" | "year") are nil off Cloud;
/// `billing` is false when the server has no Stripe settings, and then nothing offers to pay.
struct AccountPlan: Decodable, Equatable {
    let plan: String
    let until: Int64?
    let interval: String?
    /// False once a Cloud subscription is cancelled; nil from a server that predates it.
    var renews: Bool? = nil
    /// "cloud" or "plus" on a paid plan, nil otherwise and from a server that predates it.
    var planTier: String? = nil
    /// True only when the Worker can sell Plus (its Plus price ids are set); nil from an older Worker.
    var plusAvailable: Bool? = nil
    let billing: Bool

    enum CodingKeys: String, CodingKey {
        case plan, until, interval, renews, billing
        case planTier = "plan_tier"
        case plusAvailable = "plus_available"
    }

    var isCloud: Bool { plan == "cloud" }
    /// A paid plan whose tier is Plus (a Plus account is also `isCloud`).
    var isPlus: Bool { isCloud && planTier == "plus" }
}

/// `POST /v1/ai/balance`: the Whimbrel AI credit left and the newest purchase, in
/// micro-dollars (1,000,000 = $1).
struct AIBalance: Decodable, Equatable {
    let balanceMicros: Int64
    let lastPurchaseMicros: Int64?
    /// The Plus monthly allowance left; 0 when there is none, it expired, or the server predates Plus.
    let allowanceMicros: Int64
    /// When the allowance resets (ms). Only read while `allowanceMicros > 0`.
    let allowanceExpiresMs: Int64?
    /// "cloud" or "plus" on a paid plan; nil otherwise and from an older server.
    let planTier: String?

    enum CodingKeys: String, CodingKey {
        case balanceMicros = "balance_micros"
        case lastPurchaseMicros = "last_purchase_micros"
        case allowanceMicros = "allowance_micros"
        case allowanceExpiresMs = "allowance_expires_ms"
        case planTier = "plan_tier"
    }

    init(balanceMicros: Int64, lastPurchaseMicros: Int64? = nil, allowanceMicros: Int64 = 0,
         allowanceExpiresMs: Int64? = nil, planTier: String? = nil) {
        self.balanceMicros = balanceMicros
        self.lastPurchaseMicros = lastPurchaseMicros
        self.allowanceMicros = allowanceMicros
        self.allowanceExpiresMs = allowanceExpiresMs
        self.planTier = planTier
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        balanceMicros = try c.decode(Int64.self, forKey: .balanceMicros)
        lastPurchaseMicros = try c.decodeIfPresent(Int64.self, forKey: .lastPurchaseMicros)
        allowanceMicros = try c.decodeIfPresent(Int64.self, forKey: .allowanceMicros) ?? 0
        allowanceExpiresMs = try c.decodeIfPresent(Int64.self, forKey: .allowanceExpiresMs)
        planTier = try c.decodeIfPresent(String.self, forKey: .planTier)
    }

    var isPlus: Bool { planTier == "plus" }

    /// What the next AI call can draw on: prepaid credit plus any allowance.
    var spendableMicros: Int64 { balanceMicros + allowanceMicros }

    /// The reset date, only while there is allowance left (a stale date is never shown), and
    /// never one in the past.
    func allowanceResetDate(now: Date = Date()) -> Date? {
        guard allowanceMicros > 0, let ms = allowanceExpiresMs else { return nil }
        let date = Date(timeIntervalSince1970: Double(ms) / 1000)
        return date > now ? date : nil
    }

    /// Under 10% of the newest purchase is left.
    var isLow: Bool {
        guard let last = lastPurchaseMicros, last > 0 else { return false }
        return balanceMicros * 10 < last
    }
}

/// `POST /v1/ai/models`: the models Whimbrel AI offers, newest first. Every id is one the
/// server prices; `display_name` is decoded but the pickers show ids, as for the others.
struct AIModelList: Decodable, Equatable {
    struct Model: Decodable, Equatable {
        let id: String
        let displayName: String?

        enum CodingKeys: String, CodingKey {
            case id
            case displayName = "display_name"
        }
    }

    let models: [Model]
}

/// The billing routes `SyncState` calls: `SyncBillingClient` in the app, a fake in tests.
/// Mac-only, so `SyncWorker` (shared with the iPhone) stays as it is.
protocol SyncBilling: AnyObject {
    func accountPlan(idToken: String, devicePublicKey: String) async throws -> AccountPlan
    /// `plan` is "plus" for Plus; nil for Cloud, where the key is left out of the request.
    func checkoutURL(idToken: String, devicePublicKey: String, interval: String, plan: String?) async throws -> URL
    func portalURL(idToken: String, devicePublicKey: String) async throws -> URL
    func aiBalance(idToken: String, devicePublicKey: String) async throws -> AIBalance
    func aiModels(idToken: String, devicePublicKey: String) async throws -> [String]
    /// `pack` is "10" or "25" (dollars).
    func aiCheckoutURL(idToken: String, devicePublicKey: String, pack: String) async throws -> URL
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

    func checkoutURL(idToken: String, devicePublicKey: String, interval: String, plan: String?) async throws -> URL {
        if let plan {
            // The core's checkout call has no plan argument in its C interface, so Plus POSTs directly,
            // as the AI routes do. Cloud (plan nil) keeps the core path, which sends no `plan` key.
            let data = try await post("v1/billing/checkout",
                                      ["idToken": idToken, "devicePublicKey": devicePublicKey, "interval": interval, "plan": plan])
            guard let reply = try? JSONDecoder().decode(URLReply.self, from: data) else { throw SyncAPIError.malformedResponse }
            return try reply.parsed.get()
        }
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

    // The Whimbrel AI routes have no core call yet, so these POST directly. Same body and
    // the same `SyncAPIError` mapping as the billing routes above. The token is never logged.

    func aiBalance(idToken: String, devicePublicKey: String) async throws -> AIBalance {
        let data = try await post("v1/ai/balance", ["idToken": idToken, "devicePublicKey": devicePublicKey])
        guard let balance = try? JSONDecoder().decode(AIBalance.self, from: data) else { throw SyncAPIError.malformedResponse }
        return balance
    }

    func aiModels(idToken: String, devicePublicKey: String) async throws -> [String] {
        let data = try await post("v1/ai/models", ["idToken": idToken, "devicePublicKey": devicePublicKey])
        guard let list = try? JSONDecoder().decode(AIModelList.self, from: data) else { throw SyncAPIError.malformedResponse }
        return list.models.map(\.id).filter { !$0.isEmpty }
    }

    func aiCheckoutURL(idToken: String, devicePublicKey: String, pack: String) async throws -> URL {
        let data = try await post("v1/ai/checkout", ["idToken": idToken, "devicePublicKey": devicePublicKey, "pack": pack])
        guard let reply = try? JSONDecoder().decode(URLReply.self, from: data) else { throw SyncAPIError.malformedResponse }
        return try reply.parsed.get()
    }

    private func post(_ path: String, _ body: [String: String]) async throws -> Data {
        var request = URLRequest(url: baseURL.appendingPathComponent(path))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.timeoutInterval = 30
        request.httpBody = try JSONEncoder().encode(body)
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            throw SyncAPIError.networkError(error)
        }
        guard let http = response as? HTTPURLResponse else { throw SyncAPIError.malformedResponse }
        guard http.statusCode == 200 else {
            throw SyncAPIError.httpError(status: http.statusCode, body: String(decoding: data, as: UTF8.self))
        }
        return data
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
    /// "Free: 1 computer + 1 phone", "Cloud until November 9, 2027", or after a cancel
    /// "Cloud ends November 9, 2027. It won't renew." (in the user's locale).
    static func line(_ plan: AccountPlan) -> String {
        guard plan.isCloud, let until = plan.until else { return String(localized: "Free: 1 computer + 1 phone") }
        let date = Date(timeIntervalSince1970: Double(until) / 1000).formatted(.dateTime.month(.wide).day().year())
        if plan.renews == false { return String(localized: "Cloud ends \(date). It won't renew.") }
        return String(localized: "Cloud until \(date)")
    }
}
