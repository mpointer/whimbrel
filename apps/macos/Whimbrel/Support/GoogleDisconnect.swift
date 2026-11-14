import Foundation

/// What removing a Google account (Gmail or Calendar connector) does, in one place so the
/// confirmation text and the code agree: stored tokens are deleted, Whimbrel's access is
/// revoked with Google when nothing else on this Mac still uses the same grant, and the
/// account's Gmail or Calendar data is deleted from this Mac.
enum GoogleDisconnect {
    enum Service: Equatable {
        case gmail
        case calendar
    }

    /// Whether another Whimbrel connection on this Mac signs in as the same Google address,
    /// and so shares its grant with Google. Revoking would cut that connection off too, so
    /// the revoke waits until the last one is removed. Addresses compare case-insensitively.
    /// `syncEmail` is the Whimbrel sync sign-in's address, when it has one.
    static func grantIsShared(
        removing service: Service, email: String,
        gmailEmails: [String], calendarEmails: [String], syncEmail: String?
    ) -> Bool {
        let address = email.lowercased()
        let others = (service == .gmail ? calendarEmails : gmailEmails) + [syncEmail].compactMap { $0 }
        return others.contains { $0.lowercased() == address }
    }

    static var confirmationTitle: String { String(localized: "Remove this Google account?") }

    /// What the confirmation says. `revokes` is false only when the grant is shared.
    static func confirmationMessage(for service: Service, revokes: Bool) -> String {
        switch (service, revokes) {
        case (.gmail, true):
            return String(localized: "This removes this account's Gmail data from this Mac and revokes Whimbrel's access to the account with Google.")
        case (.gmail, false):
            return String(localized: "This removes this account's Gmail data from this Mac. Whimbrel keeps its Google access to this address while another Whimbrel connection to it remains.")
        case (.calendar, true):
            return String(localized: "This removes this account's Calendar events from this Mac and revokes Whimbrel's access to the account with Google. Meetings you recorded are kept.")
        case (.calendar, false):
            return String(localized: "This removes this account's Calendar events from this Mac. Whimbrel keeps its Google access to this address while another Whimbrel connection to it remains. Meetings you recorded are kept.")
        }
    }

    /// The local sign-out, then the best-effort revoke. `deleteLocal` always runs first, never
    /// waits on the network and is not undone by anything the revoke does. `startRevoke`
    /// gets the refresh token only when a revoke is due (`revoke` is true and a token was
    /// stored); the caller runs it off the main actor, since the call BLOCKS for up to 30 s.
    /// Returns why no revoke was started, as a reason code to log, or nil when one was. The
    /// token is never returned or logged.
    @discardableResult
    static func disconnect(
        refreshToken: String?, revoke: Bool, deleteLocal: () -> Void, startRevoke: (String) -> Void
    ) -> String? {
        deleteLocal()
        guard revoke else { return "grant_shared" }
        guard let refreshToken, !refreshToken.isEmpty else { return "nothing_to_revoke" }
        startRevoke(refreshToken)
        return nil
    }

    /// The revoke itself: Google's answer as a reason code ("revoked", "already_invalid",
    /// "rejected", "unreachable"), never the token. `revoker` is the core's call in the app.
    static func revokeNow(_ token: String, revoker: (String) -> String? = CoreMemoryStore.oauthRevokeGoogleToken) -> String {
        revoker(token) ?? "revoke_unavailable"
    }

    /// The address the Whimbrel sync sign-in used, read from its stored ID token without any
    /// network call. nil when sync was never signed in on this Mac.
    static func syncSignInEmail() -> String? {
        let helper = GoogleOAuthHelper(clientId: "", clientSecret: "", scope: "", keychainAccount: "google-sync")
        guard let idToken = try? helper.storedIdTokenForMigration() else { return nil }
        return GoogleOAuthHelper.decodeJwtEmailClaim(idToken)
    }
}
