import Foundation

/// A core message as `{"code","params","message"}`: the stable code and params the shell
/// renders from the String Catalog, beside core's English `message`. An unknown code, or a
/// known one missing a param, shows `message`.
struct CoreUserMessage: Decodable, Equatable {
    let code: String?
    let params: [String: CoreParam]?
    let message: String

    func localized(locale: Locale = .current) -> String {
        guard let code else { return message }
        let p = params ?? [:]
        func str(_ key: String) -> String? {
            switch p[key] {
            case let .string(s)?: return s
            case let .int(n)?: return String(n)
            default: return nil
            }
        }
        switch code {
        case "device_revoked": return String(localized: "This device's access was revoked. Sign in again from Settings to request access.")
        case "device_not_approved": return String(localized: "This device isn't approved on this account yet.")
        case "approve_on_other_device_unnamed": return String(localized: "Approve this computer on your other device; it was active in the last week.")
        case "approve_on_other_device":
            guard case let .list(names)? = p["devices"], !names.isEmpty else { return message }
            let devices = names.formatted(.list(type: .or).locale(locale))
            return String(localized: "Approve this computer on \(devices); it was active in the last week.")
        case "reset_requires_computer": return String(localized: "Starting fresh has to be done from a computer.")
        case "account_not_allowed": return String(localized: "This Google account isn't allowed to sign in to Whimbrel.")
        case "signin_expired": return String(localized: "Your Google sign-in expired. Try signing in again.")
        case "device_limit": return String(localized: "Your free sync plan includes one computer and one phone, and they're already in use.")
        case "rate_limited": return String(localized: "Too many sign-in attempts. Try again in a few minutes.")
        case "account_changing": return String(localized: "Your sync account is being changed. Try again in a few minutes.")
        case "swap_kind_mismatch": return String(localized: "That device can't take the place of this one.")
        case "wants_required": return String(localized: "Pick at least one option to join the waitlist.")
        case "reset_confirm_required": return String(localized: "Confirm that you want to start fresh to continue.")
        case "device_not_found": return String(localized: "That device is no longer on this account.")
        case "server_error":
            guard let status = str("status") else { return message }
            return String(localized: "Sync is temporarily unavailable (server error \(status)).")
        case "network": return String(localized: "Couldn't reach the sync server. Check your connection and try again.")
        case "malformed_response": return String(localized: "The sync server returned an unexpected response. Try again.")
        case "key_refused":
            guard let provider = str("provider") else { return message }
            return String(localized: "\(provider) refused this key. Check it on \(provider)'s key page.")
        case "key_unreachable":
            guard let provider = str("provider") else { return message }
            return String(localized: "Couldn't reach \(provider). The key is saved; models will load when you're online.")
        case "action_retryable":
            guard let reason = str("reason") else { return message }
            return String(localized: "Couldn't complete this action just now (\(reason)). It's still pending -- try Approve again in a bit.")
        case "connector_not_connected":
            switch str("connector") {
            case "gmail": return String(localized: "Gmail isn't connected, or needs to be reconnected. Check Settings → Connectors, then try Approve again.")
            case "google-calendar": return String(localized: "Calendar isn't connected, or needs to be reconnected. Check Settings → Connectors, then try Approve again.")
            default: return message
            }
        case "google_free_tier_training":
            return String(localized: "On Google's free tier, prompts and responses may be used to improve Google's products. The paid tier does not use them that way. Check Google's terms for the current wording.")
        case "openrouter_free_logging":
            return String(localized: "Free models on OpenRouter may log prompts. Paid models follow each upstream provider's policy. Check OpenRouter's privacy page for the current wording.")
        default: return message
        }
    }
}

enum CoreParam: Decodable, Equatable {
    case string(String), int(Int), list([String])

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let n = try? c.decode(Int.self) { self = .int(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else { self = .list(try c.decode([String].self)) }
    }
}
