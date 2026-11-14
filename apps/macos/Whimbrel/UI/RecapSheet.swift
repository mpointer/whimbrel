import AppKit
import SwiftUI

/// The pure parts of the Send recap sheet: the subject and which addresses are ticked.
enum RecapDraft {
    /// "Recap: <title> (<date>)", the date in the user's own short style.
    static func subject(title: String, startUnixMs: Int64, locale: Locale = .current) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(startUnixMs) / 1000)
        let day = date.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted).locale(locale))
        let name = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return String(localized: "Meeting recap: \(name) (\(day))")
    }

    /// The addresses to send to: ticked invitees, then added ones, each once.
    static func addresses(invitees: [CoreAttendee], ticked: Set<String>, added: [String]) -> [String] {
        var seen = Set<String>()
        return (invitees.compactMap(\.email).filter { ticked.contains($0) } + added)
            .filter { seen.insert($0.lowercased()).inserted }
    }
}

/// Send a meeting's recap to its attendees. With Gmail connected it sends from that account on
/// the user's click; otherwise it opens the draft in the default mail app. Never sends by itself.
struct RecapSheet: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss
    let meetingId: Int64
    let title: String
    let startUnixMs: Int64
    let summary: CoreRecapSummary
    /// Called with the number of people after a successful Gmail send.
    let onSent: (Int) -> Void

    @State private var invitees: [CoreAttendee] = []
    @State private var ticked: Set<String> = []
    @State private var added: [String] = []
    @State private var newAddress = ""
    @State private var subject = ""
    @State private var messageBody = ""
    @State private var isSending = false
    @State private var errorMessage: String?
    @State private var loaded = false

    private var gmailAccount: String? { GmailSelfSend.recipient(connectedAccount: appState.gmailConnectorState.defaultEmail) }
    private var addresses: [String] { RecapDraft.addresses(invitees: invitees, ticked: ticked, added: added) }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Send recap").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            recipientsSection
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text("Email subject").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                TextField("", text: $subject).textFieldStyle(.roundedBorder)
            }
            TextEditor(text: $messageBody)
                .font(.whimbrelBody(.body))
                .frame(minHeight: 220)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.whimbrelBorder))
            if let gmailAccount {
                Text("Sends from \(gmailAccount).").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                Text("Gmail isn't connected, so this opens in your mail app.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if let errorMessage {
                Text(errorMessage).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                if isSending { ProgressView().controlSize(.small) }
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction).disabled(isSending)
                Button(gmailAccount == nil ? "Open in Mail" : "Send") { send() }
                    .buttonStyle(.whimbrelPrimary)
                    .keyboardShortcut(.defaultAction)
                    .disabled(isSending || addresses.isEmpty || messageBody.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(Spacing.lg)
        .frame(minWidth: 520, minHeight: 520)
        .onAppear(perform: load)
    }

    @ViewBuilder
    private var recipientsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text("Recipients").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            if loaded && invitees.isEmpty && added.isEmpty {
                Text("No invitee has an email address. Add one below.")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            ForEach(invitees, id: \.email) { person in
                if let email = person.email {
                    Toggle(isOn: Binding(
                        get: { ticked.contains(email) },
                        set: { on in if on { ticked.insert(email) } else { ticked.remove(email) } }
                    )) {
                        Text(person.name.isEmpty || person.name == email ? email : "\(person.name) <\(email)>")
                    }
                    .toggleStyle(.checkbox)
                }
            }
            ForEach(added, id: \.self) { email in
                HStack {
                    Text(email)
                    Button {
                        added.removeAll { $0 == email }
                    } label: {
                        Image(systemName: "xmark.circle.fill").accessibilityLabel(Text("Remove"))
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
            HStack {
                TextField("Add an email address", text: $newAddress)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(addAddress)
                Button("Add", action: addAddress)
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(!RecapSender.isSendable(newAddress))
            }
        }
    }

    private func addAddress() {
        let email = newAddress.trimmingCharacters(in: .whitespaces)
        guard RecapSender.isSendable(email) else { return }
        if let invitee = invitees.first(where: { $0.email?.lowercased() == email.lowercased() }), let known = invitee.email {
            ticked.insert(known)
        } else if !added.contains(where: { $0.lowercased() == email.lowercased() }) {
            added.append(email)
        }
        newAddress = ""
    }

    private func load() {
        guard !loaded else { return }
        subject = RecapDraft.subject(title: title, startUnixMs: startUnixMs)
        messageBody = CoreMemoryStore.recapRender(summary, labels: .localized, format: "email") ?? ""
        guard let store = appState.coreStore else { loaded = true; return }
        let selfEmails = appState.recapSelfEmails
        let meetingId = meetingId
        CoreMemoryStore.accessQueue.async {
            let people = store.recapRecipients(meetingId: meetingId, selfEmails: selfEmails) ?? []
            DispatchQueue.main.async {
                invitees = people
                ticked = Set(people.compactMap(\.email))
                loaded = true
            }
        }
    }

    private func send() {
        errorMessage = nil
        let to = addresses
        guard gmailAccount != nil else {
            openInMail(to: to)
            return
        }
        isSending = true
        let sender = appState.makeRecapSender()
        Task { @MainActor in
            let result = await sender.send(to: to, subject: subject, body: messageBody)
            isSending = false
            switch result {
            case .success:
                if let store = appState.coreStore {
                    let meetingId = meetingId, count = to.count
                    let now = Int64(Date().timeIntervalSince1970 * 1000)
                    CoreMemoryStore.accessQueue.async { store.recordRecapSent(meetingId: meetingId, recipientCount: count, nowUnixMs: now) }
                }
                onSent(to.count)
                dismiss()
            case .failure(let error):
                errorMessage = String(localized: "Couldn't send the recap: \(error.reason)")
            }
        }
    }

    /// The default mail app, through `mailto:`. A body too long for the URL goes on the clipboard.
    private func openInMail(to: [String]) {
        guard let draft = CoreMemoryStore.recapMailto(to: to, subject: subject, body: messageBody, pasteHint: String(localized: "Paste the recap here.")),
              let url = URL(string: draft.url) else {
            errorMessage = String(localized: "Couldn't build the email.")
            return
        }
        if !draft.bodyInUrl {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(messageBody, forType: .string)
        }
        if NSWorkspace.shared.open(url) {
            dismiss()
        } else {
            errorMessage = String(localized: "No mail app opened. The recap is on the clipboard.")
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(messageBody, forType: .string)
        }
    }
}
