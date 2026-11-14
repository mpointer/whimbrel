import Foundation

/// Sends a meeting's recap from the default connected Gmail account to the recipients the user
/// ticked, through the same `send_email` connector call as `GmailSelfSend`. Only ever called from
/// the Send recap sheet's button: nothing sends a recap on its own.
@MainActor
struct RecapSender {
    let connectedAccount: () -> String?
    /// An access token for that account carrying the `gmail.send` scope; nil when unusable.
    let token: (String) async -> String?
    let transport: (CoreWriteActionPlan, String) async -> CoreWriteActionOutcome

    /// The `To:` value: usable addresses only, comma-separated, each once.
    nonisolated static func toHeader(_ recipients: [String]) -> String? {
        var seen = Set<String>()
        let usable = recipients
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter(RecapSender.isSendable)
            .filter { seen.insert($0.lowercased()).inserted }
        return usable.isEmpty ? nil : usable.joined(separator: ", ")
    }

    /// Mirrors core's `api::recap::is_sendable_address`: one `@`, nothing that could end the header.
    nonisolated static func isSendable(_ email: String) -> Bool {
        let e = email.trimmingCharacters(in: .whitespaces)
        guard e.count <= 254, e.filter({ $0 == "@" }).count == 1, !e.hasPrefix("@"), !e.hasSuffix("@") else { return false }
        return !e.unicodeScalars.contains { CharacterSet.whitespacesAndNewlines.contains($0) || CharacterSet.controlCharacters.contains($0) || ",;<>\"()".unicodeScalars.contains($0) }
    }

    func send(to recipients: [String], subject: String, body: String) async -> Result<Void, SelfSendError> {
        guard let from = GmailSelfSend.recipient(connectedAccount: connectedAccount()) else { return .failure(.notConnected) }
        guard let to = Self.toHeader(recipients) else { return .failure(.failed(String(localized: "Add at least one email address."))) }
        guard let token = await token(from) else { return .failure(.noToken) }
        struct Args: Encodable { let to, subject, body: String }
        let oneLineSubject = subject.components(separatedBy: .newlines).joined(separator: " ")
        guard let data = try? JSONEncoder().encode(Args(to: to, subject: oneLineSubject, body: body)) else {
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
