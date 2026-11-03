import Foundation

enum SelfSendError: Error, Equatable {
    case notConnected
    case noToken
    case failed(String)

    /// Shown on the meeting page as "Email not sent: <reason>" (stored as `failed:<reason>`).
    var reason: String {
        switch self {
        case .notConnected: return String(localized: "Gmail isn't connected.")
        case .noToken: return String(localized: "Gmail needs reconnecting in Settings to send.")
        case .failed(let reason): return reason
        }
    }
}

/// Sends an email to the user's own connected Gmail address, with no approval step
/// (live question alerts). The recipient is the connected account and nothing else: no
/// parameter or model output can change it. Goes through the same `send_email` MCP call as
/// approved routine emails (`RoutineScheduler.performToolCall`).
@MainActor
struct GmailSelfSend {
    let connectedAccount: () -> String?
    /// An access token for that account carrying the `gmail.send` scope; nil when unusable.
    let token: (String) async -> String?
    let transport: (CoreWriteActionPlan, String) async -> CoreWriteActionOutcome

    static func recipient(connectedAccount: String?) -> String? {
        guard let email = connectedAccount?.trimmingCharacters(in: .whitespaces), email.contains("@") else { return nil }
        return email
    }

    func send(subject: String, body: String) async -> Result<Void, SelfSendError> {
        guard let to = Self.recipient(connectedAccount: connectedAccount()) else { return .failure(.notConnected) }
        guard let token = await token(to) else { return .failure(.noToken) }
        struct Args: Encodable { let to, subject, body: String }
        guard let data = try? JSONEncoder().encode(Args(to: to, subject: subject, body: body)) else {
            return .failure(.failed(String(localized: "Couldn't build the email.")))
        }
        let plan = CoreWriteActionPlan(
            runId: -1, connector: "gmail", tool: "send_email", argsJson: String(decoding: data, as: UTF8.self),
            tokenEnv: "GMAIL_ACCESS_TOKEN", noTokenMessage: "", successSummary: "", terminalFailureSummary: ""
        )
        switch await transport(plan, token) {
        case .success: return .success(())
        case .noToken: return .failure(.noToken)
        case .retryableFailure(let reason): return .failure(.failed(reason))
        case .terminalFailure: return .failure(.failed(String(localized: "Google rejected the email send request.")))
        }
    }
}
