import SwiftUI

/// A transcript row's speaker: the name as shown (a guess as "Sam?" in secondary text) and,
/// when `offered`, a menu to confirm or fix it. `onConfirm` gets the guessed person's key;
/// `onPick` gets a name and, for an invitee, their email.
struct SpeakerMenu: View {
    let shown: CoreDisplayLabel
    let invitees: [CoreAttendee]
    let offered: Bool
    let onConfirm: (_ personKey: String) -> Void
    let onPick: (_ name: String, _ email: String?) -> Void

    @State private var typedName: String?

    var body: some View {
        if offered {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Menu {
                    ForEach(Array(MeetingDetailLogic.speakerMenuItems(shown: shown, invitees: invitees).enumerated()), id: \.offset) { _, item in
                        switch item {
                        case .confirm(let name, let personKey):
                            Button("Yes, that's \(name)") { onConfirm(personKey) }
                            Divider()
                        case .invitee(let person):
                            Button(person.name) { onPick(person.name, person.email) }
                        case .someoneElse:
                            Button("Someone else…") { typedName = "" }
                        }
                    }
                } label: {
                    label
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                if let name = typedName {
                    HStack(spacing: Spacing.xs) {
                        TextField("Name", text: Binding(get: { name }, set: { typedName = $0 }))
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 200)
                            .onSubmit(saveTyped)
                        Button("Save", action: saveTyped)
                            .buttonStyle(.whimbrelPrimary)
                            .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        Button("Cancel") { typedName = nil }
                            .buttonStyle(.whimbrelSecondary)
                    }
                    .controlSize(.small)
                }
            }
        } else {
            label
        }
    }

    private var label: some View {
        Text(verbatim: MeetingDetailLogic.displaySpeaker(shown.label, guess: shown.guess))
            .font(.whimbrelBody(.caption).bold())
            .foregroundStyle(shown.guess ? Color.whimbrelTextSecondary : Color.whimbrelTextPrimary)
    }

    private func saveTyped() {
        guard let name = typedName?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty else { return }
        typedName = nil
        onPick(name, nil)
    }
}

/// Shown after a speaker's name changes: the summary keeps the old names until it is redone.
struct NamesChangedBanner: View {
    let updating: Bool
    let canUpdate: Bool
    let error: String?
    let onUpdate: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack(spacing: Spacing.sm) {
                Text("Names changed. Update the summary?")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                Spacer(minLength: 0)
                Button("Update the summary", action: onUpdate)
                    .buttonStyle(.whimbrelPrimary)
                    .controlSize(.small)
                    .disabled(updating || !canUpdate)
                if updating { ProgressView().controlSize(.small) }
            }
            if let error {
                Text(error).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelError)
            }
        }
        .padding(Spacing.sm)
        .background(Color.whimbrelHighlight)
        .clipShape(RoundedRectangle(cornerRadius: Radius.card))
    }
}
