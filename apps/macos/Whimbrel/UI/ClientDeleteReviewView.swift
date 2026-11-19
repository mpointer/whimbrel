import AppKit
import SwiftUI

/// The review screen for "Delete this client…" and "Export…" (spec 4.1, 6): Linked, Possible matches (unticked),
/// Left for you. Delete needs the client's name typed exactly; it then shows the receipt, which carries counts
/// and categories only. Owned by the Topics tab, so the receipt stays when the deleted topic leaves the list.
struct ClientDeleteReviewView: View {
    @ObservedObject var model: ClientReviewModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.lg) {
                    Text(model.mode == .delete ? "Delete this client" : "Export this client's material")
                        .font(.whimbrelDisplay(.title3))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    content
                }
                .padding(Spacing.xl)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            // The name field (Delete) and the export options stay in view however long the lists are.
            if model.phase == .review, model.inventory != nil {
                Divider().overlay(Color.whimbrelBorder)
                pinnedControls.padding(.horizontal, Spacing.xl).padding(.vertical, Spacing.md)
            }
            Divider().overlay(Color.whimbrelBorder)
            footer.padding(Spacing.lg)
        }
        .frame(width: 600, height: 640)
        .background(Color.whimbrelBackgroundPage)
        .task { await model.load() }
        .onChange(of: model.phase) { _, phase in
            if phase == .review, model.mode == .delete { nameFocused = true }
        }
        // A delete that has not started is cancelled when the sheet closes; one that has started finishes
        // and its receipt waits in the Topics tab. An export cannot be closed while it runs.
        .onDisappear { model.cancelWait() }
        .interactiveDismissDisabled(model.mode == .export && model.isRunning)
    }

    @FocusState private var nameFocused: Bool

    @ViewBuilder
    private var pinnedControls: some View {
        if model.mode == .delete { confirmField } else { exportOptions }
    }

    // MARK: Content by phase

    @ViewBuilder
    private var content: some View {
        switch model.phase {
        case .loading:
            ProgressView().controlSize(.small)
        case .review:
            if let inv = model.inventory { review(inv) }
        case .running:
            running
        case .waiting:
            waiting
        case .receipt(let r):
            VStack(alignment: .leading, spacing: Spacing.sm) {
                if model.cancelTooLate {
                    note(String(localized: "Too late to cancel. The delete had already started and finished."))
                }
                ClientReceiptView(receipt: r, retryGone: model.retryGone)
            }
        case .exported(let outcome):
            exported(outcome)
        case .gone:
            note(ClientCopy.message(.notFound))
        case .noIdentity:
            note(ClientCopy.noIdentity)
        case .failed(let e):
            note(ClientCopy.message(e))
        }
    }

    private var waiting: some View {
        HStack(alignment: .top, spacing: Spacing.sm) {
            ProgressView().controlSize(.small)
            Text("Waiting for background work to finish…")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
    }

    private var running: some View {
        HStack(alignment: .top, spacing: Spacing.sm) {
            ProgressView().controlSize(.small)
            Text(model.mode == .delete
                 ? "Deleting. Whimbrel first waits for other work on this Mac to finish, which can take a few minutes."
                 : "Exporting…")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
    }

    private func review(_ inv: ClientInventory) -> some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            if let notice = model.notice {
                note(ClientCopy.message(notice == .busy ? .busy : .selectionChanged))
            }
            group(model.mode == .delete ? "Linked: deleted" : "Linked: exported") {
                linked(inv)
            }
            group("Possible matches") { possible(inv) }
            if model.mode == .delete {
                group("Left for you") { leftForYou(inv.leftForYou) }
                Text(verbatim: ClientCopy.reappears)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private func linked(_ inv: ClientInventory) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            let counts = ClientCopy.counts(inv.counts)
            if counts.isEmpty {
                caption("Nothing is filed under this client yet.")
            } else {
                ForEach(counts, id: \.self) { Text(verbatim: $0).font(.whimbrelBody(.callout)) }
            }
            let listed = inv.linked.filter { $0.kind != .capture }
            if !listed.isEmpty {
                DisclosureGroup("Show linked items") {
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        ForEach(listed, id: \.self) { itemRow($0) }
                    }
                    .padding(.top, Spacing.xs)
                }
                .font(.whimbrelBody(.callout))
            }
            ForEach(sharedWith(inv.linked), id: \.self) { other in
                Text(model.mode == .delete
                     ? "Some of these also belong to \(other). They are deleted for both."
                     : "Some of these also belong to \(other). They are exported whole.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelWarning)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if model.mode == .delete {
                plainCaption(ClientCopy.derived)
                plainCaption(ClientCopy.gmailThreads)
            } else {
                caption("Never another client's own material. A meeting shared with another client is exported whole.")
                caption("Gmail thread summaries are not included. Possible matches are included only if you tick them.")
            }
        }
    }

    private func sharedWith(_ items: [ClientItem]) -> [String] {
        Array(Set(items.compactMap(\.alsoClient))).sorted()
    }

    @ViewBuilder
    private func possible(_ inv: ClientInventory) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            if inv.possible.isEmpty {
                caption("No other items name this client.")
            } else {
                caption(model.mode == .delete
                        ? LocalizedStringKey("Items that name this client but are not filed under it. Tick the ones to delete too.")
                        : LocalizedStringKey("Items that name this client but are not filed under it. Tick the ones to export too."))
                ForEach(ClientRowText.rows(inv.possible)) { row in
                    switch row {
                    case .item(let item):
                        Toggle(isOn: Binding(get: { model.isTicked(item) }, set: { model.setTicked(item, $0) })) {
                            itemText(item)
                        }
                        .toggleStyle(.checkbox)
                    case .group(let group):
                        captureGroup(group)
                    }
                }
                if inv.possibleTruncated {
                    caption(model.mode == .delete
                            ? LocalizedStringKey("More items name this client than are listed here. The ones not listed are not deleted and stay in Whimbrel; delete them where they are if you need to.")
                            : LocalizedStringKey("More items name this client than are listed here. The ones not listed are not exported."))
                }
            }
        }
    }

    /// A row's text: its name, then what it is and when, then the line that names the client.
    private func itemText(_ item: ClientItem) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(verbatim: ClientRowText.title(item)).font(.whimbrelBody(.callout)).lineLimit(1)
            if let detail = ClientRowText.detail(item) {
                Text(verbatim: detail).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if let excerpt = item.excerpt, !excerpt.isEmpty {
                Text(verbatim: excerpt).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary).italic()
                    .lineLimit(3).fixedSize(horizontal: false, vertical: true)
            }
            if let other = item.alsoClient {
                Text("Also belongs to \(other)").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelWarning)
            }
        }
    }

    /// A linked row: no tick (all linked items go), the same text.
    private func itemRow(_ item: ClientItem) -> some View {
        itemText(item).frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Screen captures from one app on one day: one box ticks them all; the arrow shows each with its own box.
    private func captureGroup(_ group: ClientCaptureGroup) -> some View {
        DisclosureGroup {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                ForEach(group.items, id: \.self) { item in
                    Toggle(isOn: Binding(get: { model.isTicked(item) }, set: { model.setTicked(item, $0) })) {
                        itemText(item)
                    }
                    .toggleStyle(.checkbox)
                }
            }
            .padding(.top, Spacing.xs)
        } label: {
            let n = model.tickedCount(group.items)
            Toggle(isOn: Binding(get: { model.isTicked(group.items) }, set: { model.setTicked(group.items, $0) })) {
                VStack(alignment: .leading, spacing: 0) {
                    Text(verbatim: ClientRowText.groupTitle(group)).font(.whimbrelBody(.callout))
                    if n > 0 && n < group.items.count {
                        Text(verbatim: ClientRowText.groupProgress(ticked: n, of: group.items.count))
                            .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    }
                }
            }
            .toggleStyle(.checkbox)
        }
    }

    @ViewBuilder
    private func leftForYou(_ list: [ClientLeftForYou]) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            caption("Places Whimbrel cannot clear for you.")
            ForEach(list.compactMap(ClientCopy.leftForYou), id: \.self) { line in
                LeftForYouRow(line: line)
            }
            if list.contains(where: { if case .exportFolders(let p) = $0 { return !p.isEmpty } else { return false } }) {
                Toggle("Also delete the export folders listed above", isOn: $model.deleteExports)
                    .toggleStyle(.checkbox)
            }
        }
    }

    private var confirmField: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text("Type the client's name to confirm: \(model.name)")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextPrimary)
            TextField("Client name", text: $model.confirmText)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 320)
                .focused($nameFocused)
                .onSubmit { Task { await model.delete() } }
                .accessibilityLabel(Text("Type the client's name to confirm"))
        }
    }

    private var exportOptions: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Toggle("Include audio and slide images (can be large)", isOn: $model.includeMedia)
                .toggleStyle(.checkbox)
            caption("Whimbrel makes a new folder inside the folder you choose. Export to a folder on this Mac; a network share is refused.")
        }
    }

    private func exported(_ outcome: ClientReviewModel.ExportOutcome) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Exported.").font(.whimbrelBody(.callout).bold())
            LeftForYouRow(line: .init(text: String(localized: "The export folder:"), paths: [outcome.folder]))
            if outcome.partial {
                Text("The text is complete, but some audio or slide images were not copied.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelWarning)
            }
            if let media = outcome.media {
                caption("Audio and slide images: \(media.files) copied, \(media.skipped) skipped, \(media.failed) failed.")
            }
            caption("The folder has a README that says what is in it. It is a plain copy of private data, yours to keep or delete. Time Machine or cloud sync may copy it too. If you delete this client later, Whimbrel lists this folder under Left for you and offers to delete it. If you move or rename it, Whimbrel loses track of it: delete it yourself.")
            Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: outcome.folder)]) }
                .buttonStyle(.whimbrelSecondary)
        }
    }

    // MARK: Footer

    @ViewBuilder
    private var footer: some View {
        HStack(spacing: Spacing.sm) {
            if case .receipt(let r) = model.phase {
                Button("Save receipt…") { ClientReceiptFile.save(r) }.buttonStyle(.whimbrelQuiet)
            }
            Spacer()
            switch model.phase {
            case .review where model.mode == .delete:
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Delete client and chosen items", role: .destructive) { Task { await model.delete() } }
                    .buttonStyle(.whimbrelPrimary)
                    .disabled(!model.canDelete)
                if model.notice == .busy {
                    Button("Try again") { Task { await model.retry() } }.buttonStyle(.whimbrelSecondary)
                }
            case .review:
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Choose folder and export…") { chooseFolder() }
                    .buttonStyle(.whimbrelPrimary)
                    .disabled(!model.canExport)
            case .waiting:
                Button("Cancel") { model.cancelWait() }.keyboardShortcut(.cancelAction)
            case .running where model.mode == .delete:
                Button("Cancel") { model.cancelWait() }.keyboardShortcut(.cancelAction)
                Button("Delete client and chosen items", role: .destructive) {}.buttonStyle(.whimbrelPrimary).disabled(true)
            case .running:
                EmptyView()
            case .receipt(let r):
                if model.exportFolderUnreachable && !model.retryGone {
                    Button("Run again without deleting exports") { Task { await model.retryWithoutDeletingExports() } }
                        .buttonStyle(.whimbrelSecondary)
                }
                if ClientCopy.retryHelps(r) && !model.retryGone {
                    Button("Try again") { Task { await model.retry() } }.buttonStyle(.whimbrelSecondary)
                }
                Button("Done") { dismiss() }.buttonStyle(.whimbrelPrimary).keyboardShortcut(.defaultAction)
            case .failed(let e) where model.mode == .delete && ClientCopy.isRetryable(e):
                Button("Close") { dismiss() }
                Button("Try again") { Task { await model.retry() } }.buttonStyle(.whimbrelPrimary)
            case .failed where model.mode == .export:
                Button("Close") { dismiss() }
                Button("Choose another folder…") { model.backToReview(); chooseFolder() }.buttonStyle(.whimbrelPrimary)
            default:
                Button("Close") { dismiss() }.buttonStyle(.whimbrelPrimary).keyboardShortcut(.defaultAction)
            }
        }
    }

    /// The user picks the export's parent folder in the open panel (never a path from elsewhere).
    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Export here")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await model.export(to: url) }
    }

    // MARK: Pieces

    private func group<Content: View>(_ title: LocalizedStringKey, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text(title).font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }

    private func caption(_ text: LocalizedStringKey) -> some View {
        Text(text).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary).fixedSize(horizontal: false, vertical: true)
    }

    private func plainCaption(_ text: String) -> some View {
        Text(verbatim: text).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary).fixedSize(horizontal: false, vertical: true)
    }

    private func note(_ text: String) -> some View {
        Text(verbatim: text).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelWarning).fixedSize(horizontal: false, vertical: true)
    }
}

/// One "Left for you" line and the paths it names, each with Show in Finder.
private struct LeftForYouRow: View {
    let line: ClientCopy.LeftLine

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: line.text)
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(line.paths, id: \.self) { path in
                HStack(spacing: Spacing.xs) {
                    Text(verbatim: path)
                        .font(.whimbrelBody(.caption).monospaced())
                        .foregroundStyle(Color.whimbrelTextSecondary)
                        .textSelection(.enabled)
                        .lineLimit(2)
                    Button { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)]) } label: {
                        Image(systemName: "folder")
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(Color.whimbrelLink)
                    .help(Text("Show in Finder"))
                    .accessibilityLabel(Text("Show in Finder"))
                }
            }
        }
    }
}

/// The receipt: counts and categories only, never the client's name or text (spec 4.4).
struct ClientReceiptView: View {
    let receipt: ClientDeleteReceipt
    let retryGone: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text(verbatim: ClientCopy.receiptHeadline(receipt))
                .font(.whimbrelBody(.callout).bold())
                .foregroundStyle(receipt.complete ? Color.whimbrelTextPrimary : Color.whimbrelWarning)
                .fixedSize(horizontal: false, vertical: true)
            if let failed = ClientCopy.failedLine(receipt) {
                Text(verbatim: failed).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelWarning)
            }
            if retryGone {
                Text("The client record is already gone, so there is nothing more to run again. What is left is listed below.")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            section("Deleted", ClientCopy.counts(receipt.counts))
            if !ClientCopy.counts(receipt.notSent).isEmpty {
                section("Deleted here, not sent to your other devices", ClientCopy.counts(receipt.notSent))
            }
            if !ClientCopy.counts(receipt.localOnly).isEmpty {
                section("Never synced from this Mac", ClientCopy.counts(receipt.localOnly))
            }
            // Only when that stage did not fail: the failed line above says so otherwise.
            if !receipt.failedStages.contains(.derivedText) {
                Text(verbatim: ClientCopy.derived).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text(verbatim: ClientCopy.gmailThreads).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if receipt.rollbackCopyRemoved == true {
                Text(verbatim: ClientCopy.rollbackCopyRemoved).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if receipt.client != .removedHereOnly {
                Text(verbatim: ClientCopy.pendingDevices(receipt.pendingDevices))
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text("Left for you").font(.whimbrelBody(.callout).bold()).accessibilityAddTraits(.isHeader)
                ForEach(receipt.leftForYou.compactMap(ClientCopy.leftForYou), id: \.self) { LeftForYouRow(line: $0) }
            }
            Text(verbatim: ClientCopy.reappears).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    private func section(_ title: LocalizedStringKey, _ lines: [String]) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.whimbrelBody(.callout).bold()).accessibilityAddTraits(.isHeader)
            if lines.isEmpty {
                Text("Nothing.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            ForEach(lines, id: \.self) { Text(verbatim: $0).font(.whimbrelBody(.callout)) }
        }
    }
}

/// "Save receipt…": the receipt as plain text (counts and categories, no client name). The saved file is the
/// user's to keep or delete.
enum ClientReceiptFile {
    static func text(_ r: ClientDeleteReceipt) -> String {
        var lines = [String(localized: "Whimbrel client delete receipt"),
                     Date(timeIntervalSince1970: Double(r.dateUnixMs) / 1000).formatted(date: .long, time: .shortened), "",
                     ClientCopy.receiptHeadline(r)]
        if let failed = ClientCopy.failedLine(r) { lines.append(failed) }
        lines += ["", String(localized: "Deleted")] + ClientCopy.counts(r.counts)
        if !r.notSent.isEmpty { lines += ["", String(localized: "Deleted here, not sent to your other devices")] + ClientCopy.counts(r.notSent) }
        if !r.localOnly.isEmpty { lines += ["", String(localized: "Never synced from this Mac")] + ClientCopy.counts(r.localOnly) }
        if r.client != .removedHereOnly { lines += ["", ClientCopy.pendingDevices(r.pendingDevices)] }
        lines += ["", String(localized: "Left for you")]
        // No paths: a folder name can carry a meeting title, and so the client's name.
        for l in r.leftForYou.compactMap(ClientCopy.leftForYou) {
            lines.append(l.text)
            if !l.paths.isEmpty { lines.append(String(localized: "  Places: \(l.paths.count) (shown in Whimbrel, not saved here)")) }
        }
        if !r.failedStages.contains(.derivedText) { lines += ["", ClientCopy.derived] }
        if r.rollbackCopyRemoved == true { lines.append(ClientCopy.rollbackCopyRemoved) }
        lines += [ClientCopy.gmailThreads, "", ClientCopy.reappears, String(localized: "This file is yours to keep or delete.")]
        return lines.joined(separator: "\n") + "\n"
    }

    @MainActor
    static func save(_ r: ClientDeleteReceipt) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = String(localized: "Whimbrel delete receipt.txt")
        panel.allowedContentTypes = [.plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        try? text(r).write(to: url, atomically: true, encoding: .utf8)
    }
}

/// The weekly keep-for-N-months review: each client's items past its period, ticked to delete, with Keep and
/// Snooze per item. Deletes run on this Mac only.
struct ClientRetentionReviewView: View {
    @ObservedObject var model: ClientRetentionReviewModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.lg) {
                    Text("Items ready to review").font(.whimbrelDisplay(.title3)).foregroundStyle(Color.whimbrelTextPrimary)
                    content
                }
                .padding(Spacing.xl)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            Divider().overlay(Color.whimbrelBorder)
            footer.padding(Spacing.lg)
        }
        .frame(width: 600, height: 600)
        .background(Color.whimbrelBackgroundPage)
        .task { await model.load() }
        .interactiveDismissDisabled(model.phase == .running)
    }

    @ViewBuilder
    private var content: some View {
        switch model.phase {
        case .loading:
            ProgressView().controlSize(.small)
        case .review:
            Text("These items are past their client's keep period. Ticked items are deleted from this Mac when you confirm; each of your other devices asks before it deletes its own copies. Keep never asks again about an item; Snooze asks again in 30 days.")
                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary).fixedSize(horizontal: false, vertical: true)
            if let error = model.errorText {
                Text(verbatim: error).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
            if model.groups.isEmpty {
                Text("Nothing is waiting for review.").font(.whimbrelBody(.callout))
            }
            ForEach(model.groups) { group in
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text(verbatim: group.name).font(.whimbrelBody(.callout).bold()).accessibilityAddTraits(.isHeader)
                    ForEach(group.items, id: \.self) { item in
                        HStack(spacing: Spacing.sm) {
                            Toggle(isOn: Binding(get: { model.isTicked(key: group.key, item) }, set: { model.setTicked(key: group.key, item, $0) })) {
                                VStack(alignment: .leading, spacing: 0) {
                                    Text(verbatim: ClientRowText.title(item)).font(.whimbrelBody(.callout)).lineLimit(1)
                                    if let detail = ClientRowText.detail(item) {
                                        Text(verbatim: detail).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                                    }
                                    if let other = item.alsoClient {
                                        Text("Also belongs to \(other)").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelWarning)
                                    }
                                }
                            }
                            .toggleStyle(.checkbox)
                            Spacer()
                            Button("Keep") { Task { await model.keep(key: group.key, item) } }.buttonStyle(.whimbrelQuiet)
                            Button("Snooze 30 days") { Task { await model.snooze(key: group.key, item) } }.buttonStyle(.whimbrelQuiet)
                        }
                    }
                }
            }
        case .running:
            HStack(spacing: Spacing.sm) {
                ProgressView().controlSize(.small)
                Text("Deleting on this Mac…").font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        case .done(let s):
            done(s)
        case .failed(let e):
            Text(verbatim: ClientCopy.message(e)).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelWarning)
        }
    }

    private func done(_ s: ClientExpirySummary) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text(s.complete ? "Deleted from this Mac." : "Not finished.").font(.whimbrelBody(.callout).bold())
            if let e = s.stopped { Text(verbatim: ClientCopy.expiryStopped(e)).foregroundStyle(Color.whimbrelWarning) }
            if s.unfinished { Text("Some items are still waiting. Review again to finish.").foregroundStyle(Color.whimbrelWarning) }
            ForEach(ClientCopy.counts(s.counts), id: \.self) { Text(verbatim: $0) }
            if !ClientCopy.counts(s.waitingToSync).isEmpty {
                Text("Not yet sent to your other devices, so kept for now and offered again later:").font(.whimbrelBody(.caption))
                ForEach(ClientCopy.counts(s.waitingToSync), id: \.self) { Text(verbatim: $0).font(.whimbrelBody(.caption)) }
            }
            if !ClientCopy.counts(s.notDue).isEmpty {
                Text("No longer due, so kept:").font(.whimbrelBody(.caption))
                ForEach(ClientCopy.counts(s.notDue), id: \.self) { Text(verbatim: $0).font(.whimbrelBody(.caption)) }
            }
            if !s.failed.isEmpty {
                Text(verbatim: String(localized: "Not removed: \(s.failed.map(ClientCopy.kind).joined(separator: ", ")).")).foregroundStyle(Color.whimbrelWarning)
            }
            ForEach(s.leftForYou.compactMap(ClientCopy.expiryLeftForYou), id: \.self) { LeftForYouRow(line: $0) }
            Text(verbatim: ClientCopy.gmailThreads).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .font(.whimbrelBody(.callout))
    }

    @ViewBuilder
    private var footer: some View {
        HStack(spacing: Spacing.sm) {
            Spacer()
            switch model.phase {
            case .review:
                Button("Later") { Task { await model.later(); dismiss() } }
                Button("Delete ticked items (\(model.tickedCount))", role: .destructive) { Task { await model.deleteTicked() } }
                    .buttonStyle(.whimbrelPrimary)
                    .disabled(model.tickedCount == 0)
            case .running:
                EmptyView()
            default:
                Button("Done") { dismiss() }.buttonStyle(.whimbrelPrimary).keyboardShortcut(.defaultAction)
            }
        }
    }
}
