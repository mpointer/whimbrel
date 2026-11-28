import SwiftUI

/// The "Client" section on a seed topic's page: Delete this client…, Export…, Keep for N months, Exclude from
/// hosted models. Hidden when core says the topic is not a live client.
struct ClientControlsView: View {
    @StateObject private var model: ClientSettingsModel
    /// Opens the review sheet (owned by the Topics tab, so a receipt outlives the page it came from).
    let openReview: (ClientReviewModel.Mode) -> Void

    init(key: String, bridge: ClientControlsBridge, openReview: @escaping (ClientReviewModel.Mode) -> Void) {
        _model = StateObject(wrappedValue: ClientSettingsModel(key: key, bridge: bridge))
        self.openReview = openReview
    }

    var body: some View {
        // A stack, not a Group: a Group with no content gives `.task` nothing to attach to, so the load never
        // ran and the section never appeared (0.9.80 device check, row 1).
        VStack(alignment: .leading, spacing: 0) {
            if model.isClient && model.saved != nil {
                VStack(alignment: .leading, spacing: Spacing.md) {
                    Text("Client").restyledFont(.calloutStrong).foregroundStyle(Color.whimbrelTextPrimary)
                    if let error = model.errorText {
                        Text(error).restyledFont(.caption).foregroundStyle(Color.whimbrelError)
                    }
                    actions
                    keep
                    exclude
                }
                .padding(Spacing.md)
                .overlay(RoundedRectangle(cornerRadius: Radius.control).stroke(Color.whimbrelBorder))
            } else if model.isClient, let error = model.errorText {
                // The settings could not be read: say so instead of showing nothing.
                Text(error).restyledFont(.caption).foregroundStyle(Color.whimbrelError)
            }
        }
        .task { await model.load() }
    }

    private var actions: some View {
        HStack(spacing: Spacing.sm) {
            Button("Delete this client…", role: .destructive) { openReview(.delete) }
                .restyledButton(.secondary)
                .help(Text("Review what Whimbrel holds about this client, then delete it."))
            Button("Export…") { openReview(.export) }
                .restyledButton(.secondary)
                .help(Text("Copy what Whimbrel holds about this client into a folder you choose."))
        }
    }

    // MARK: Keep for N months

    private var keep: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Picker("Keep for", selection: Binding(get: { model.keepMonths }, set: { months in Task { await model.setKeepMonths(months) } })) {
                ForEach(model.keepOptions, id: \.self) { months in
                    Text(ClientCopy.keepLabel(months)).tag(months)
                }
            }
            .pickerStyle(.menu)
            .fixedSize()
            Group {
                Text("The period syncs to your other devices, and those that have updated apply it. Each device asks before it deletes its own copies; deleting here does not delete them elsewhere. Each item's age counts from its own date (meeting end, email date, event date, capture time). An item shared with another client that has no keep period is kept.")
                Text(verbatim: ClientCopy.gmailThreads)
            }
            .restyledFont(.caption)
            .foregroundStyle(Color.whimbrelTextSecondary)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: Exclude from hosted models

    private var exclude: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack(spacing: Spacing.sm) {
                Toggle("Exclude from hosted models", isOn: Binding(get: { model.excludeHosted }, set: { on in Task { await model.setExcludeHosted(on) } }))
                    .toggleStyle(.switch)
                if model.isSaving {
                    ProgressView().controlSize(.small)
                    Text("Saving…").restyledFont(.caption).foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
            Group {
                Text("Material that names or involves this client is left out of what Whimbrel sends to hosted AI models. A request that names the client is answered by a local model on this Mac, or refused when there is none.")
                Text("Whimbrel treats a recording as this client's when it comes from or overlaps one of the client's calendar events, names the client, includes one of the client's contacts, or you file it under the client. A call with none of these looks like any other recording until you file it.")
                Text("An item shared with another client is left out if either client is excluded. Colleagues who mainly email this client may be left out too. A calendar series once filed under this client stays left out after you unfile it.")
                Text(verbatim: ClientCopy.matchingLimit)
            }
            .restyledFont(.caption)
            .foregroundStyle(Color.whimbrelTextSecondary)
            .fixedSize(horizontal: false, vertical: true)
            if let held = ClientCopy.heldDevices(model.heldDevices) {
                Text(verbatim: held)
                    .restyledFont(.caption)
                    .foregroundStyle(Color.whimbrelWarning)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let notice = model.exclusionNotice {
                exclusionNotice(notice)
            }
        }
    }

    private func exclusionNotice(_ notice: ClientSettingsModel.ExclusionNotice) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text(verbatim: notice.alreadyLeft)
                .restyledFont(.callout)
                .foregroundStyle(Color.whimbrelTextPrimary)
            if !notice.hostedSummaries.isEmpty {
                Text("A hosted model summarised \(notice.hostedSummaries.count) of this client's meetings before. Those summaries stay as they are; from now on they are left out of what Whimbrel sends.")
                    .restyledFont(.caption)
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if notice.resummaryStarted {
                    Text("Summarising them again on this Mac. Without a local model they are not made again.")
                        .restyledFont(.caption)
                        .foregroundStyle(Color.whimbrelTextSecondary)
                } else {
                    Button("Summarise them again with a local model") { model.resummarize() }
                        .restyledButton(.quiet)
                }
            }
        }
        .padding(Spacing.sm)
        .background(Color.whimbrelHighlight)
        .clipShape(RoundedRectangle(cornerRadius: Radius.control))
    }
}

/// The weekly "items ready to review" banner on the Topics tab (keep for N months).
struct ClientReviewBanner: View {
    let flag: ClientReviewFlag
    let review: () -> Void
    let later: () -> Void

    var body: some View {
        HStack(spacing: Spacing.md) {
            Image(systemName: "clock.badge.exclamationmark").foregroundStyle(Color.whimbrelWarning)
                .accessibilityHidden(true)
            Text("Items past their keep period are ready to review (items: \(flag.items), clients: \(flag.clients)). Nothing is deleted until you confirm.")
                .restyledFont(.callout)
                .foregroundStyle(Color.whimbrelTextPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer()
            Button("Review…", action: review).restyledButton(.primary)
            Button("Later", action: later).restyledButton(.quiet)
        }
        .padding(Spacing.md)
        .background(Color.whimbrelBackgroundCard)
        .accessibilityElement(children: .contain)
    }
}
