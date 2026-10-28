import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Settings → Appearance: make or change a skin by talking to Whimbrel. Everything lives in the
/// long-lived `session` (owned by `AppState`), because each skin the chat switches on rebuilds
/// the Settings window (`ThemedRoot`) and this view with it.
struct SkinChatView: View {
    @ObservedObject var session: SkinChatSession
    /// Opens the AI settings, for when no provider is usable.
    let setUp: () -> Void

    /// The chat's scroll id: a chat action brings the rebuilt Settings window back here.
    static let scrollId = "skin-chat"

    static func placeholder(for skin: Skin) -> String {
        SkinGallery.isUserSkin(skin) ? "Ask for a change…" : "Describe a skin…"
    }

    private var canSend: Bool {
        !session.isRunning && !session.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            ForEach(Array(session.turns.enumerated()), id: \.offset) { _, turn in
                bubble(turn)
            }
            if !session.status.isEmpty { statusList }
            if !session.candidates.isEmpty { candidateStrip }
            if session.available { inputRow } else { SetUpAIProviderButton(action: setUp) }
            HStack(spacing: Spacing.sm) {
                Button("Add your own image…", action: pickImage)
                    .buttonStyle(.whimbrelSecondary)
                Button("Undo") { session.undo() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(session.isRunning)
            }
        }
        .id(Self.scrollId)
    }

    private func bubble(_ turn: SkinChatTurn) -> some View {
        let mine = turn.role == "user"
        return Text(turn.text)
            .font(.whimbrelBody(.callout))
            .foregroundStyle(mine ? Color.whimbrelTextOnAccent : Color.whimbrelTextPrimary)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, Spacing.md)
            .padding(.vertical, Spacing.sm)
            .background(mine ? Color.whimbrelAccentPrimary : Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
            .frame(maxWidth: .infinity, alignment: mine ? .trailing : .leading)
    }

    /// What the current (or last) turn did, step by step; Stop while it runs.
    private var statusList: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            ForEach(Array(session.status.enumerated()), id: \.offset) { index, line in
                HStack(spacing: Spacing.xs) {
                    if session.isRunning && index == session.status.count - 1 {
                        ProgressView().controlSize(.mini)
                    }
                    Text(line)
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if session.isRunning {
                Button("Stop") { session.stop() }
                    .buttonStyle(.whimbrelSecondary)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Progress")
    }

    private var candidateStrip: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text("Found these").font(.whimbrelBody(.caption).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: Spacing.sm) {
                    ForEach(session.candidates) { candidate in
                        CandidateTile(candidate: candidate, session: session)
                    }
                }
            }
        }
    }

    private var inputRow: some View {
        HStack(spacing: Spacing.sm) {
            TextField(Self.placeholder(for: SkinStore.shared.selected), text: $session.draft)
                .textFieldStyle(.roundedBorder)
                .font(.whimbrelBody(.body))
                .onSubmit(send)
            Button("Send", action: send)
                .buttonStyle(.whimbrelPrimary)
                .disabled(!canSend)
        }
    }

    private func send() {
        guard canSend else { return }
        let text = session.draft
        session.draft = ""
        session.send(text)
    }

    private func pickImage() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.png, .jpeg, .gif, .webP]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        session.addOwnImage(url)
    }
}

/// A found or generated image with Approve and Reject. The thumbnail comes only from
/// `SkinChatSession.thumbnail(for:)`, which fetches through the core's guarded fetch (R10).
private struct CandidateTile: View {
    let candidate: ImageCandidateDTO
    @ObservedObject var session: SkinChatSession
    @State private var image: NSImage?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Group {
                if let image {
                    Image(nsImage: image).resizable().scaledToFill()
                } else {
                    Color.whimbrelBackgroundCard
                }
            }
            .frame(width: 176, height: 116)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.whimbrelBorder, lineWidth: 0.5))
            .accessibilityLabel(candidate.title)
            Text(candidate.title)
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .lineLimit(1)
            HStack(spacing: Spacing.xs) {
                Button("Approve") {
                    session.approve(candidate)
                }
                .buttonStyle(.whimbrelPrimary)
                Button("Reject") { session.reject(candidate) }
                    .buttonStyle(.whimbrelSecondary)
            }
            .fixedSize()
        }
        .frame(width: 176)
        .task(id: candidate.id) { image = await session.thumbnail(for: candidate) }
    }
}
