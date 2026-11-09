import SwiftUI

struct TopicSeed: Decodable, Identifiable, Equatable {
    let topicId: Int64
    let key: String
    let name: String
    let aliases: [String]
    let kind: String?
    let itemCount: Int
    var id: String { key }

    enum CodingKeys: String, CodingKey {
        case topicId = "topic_id", key, name, aliases, kind, itemCount = "item_count"
    }
}

struct TopicSuggestion: Decodable, Identifiable, Equatable {
    let topicId: Int64
    let name: String
    let itemCount: Int
    var id: Int64 { topicId }

    enum CodingKeys: String, CodingKey { case topicId = "topic_id", name, itemCount = "item_count" }
}

/// Settings > Topics: the topics the user named (with aliases and a kind) and the ones
/// Whimbrel suggests.
struct TopicsSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var seeds: [TopicSeed] = []
    @State private var suggestions: [TopicSuggestion] = []
    @State private var editor: TopicEditorTarget?
    @State private var deletingKey: String?
    @State private var mergingKey: String?
    @State private var mergeInto = ""
    @State private var suggestionNames: [Int64: String] = [:]
    @State private var errorText: String?
    @State private var names: [String] = []
    @State private var suggestedNames: [String] = []
    @State private var newName = ""


    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Topics").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Name the customers, products and projects you care about. Whimbrel groups your meetings, emails and notes under them.")
                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            if let errorText {
                Text(errorText).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
            yoursSection
            Divider().overlay(Color.whimbrelBorder)
            suggestedSection
            Divider().overlay(Color.whimbrelBorder)
            namesSection
            Divider().overlay(Color.whimbrelBorder)
            stepsSection
        }
        .task {
            // Reload on appear and every 30 s; the task is cancelled when the pane goes away.
            while !Task.isCancelled {
                reload()
                try? await Task.sleep(for: .seconds(30))
            }
        }
        .sheet(item: $editor) { target in
            TopicEditorSheet(target: target) { name, aliases, kind in
                run({ $0.topicSeedSave(key: target.seed?.key, name: name, aliases: aliases, kind: kind) },
                    failure: "Could not save that topic.")
            }
        }
    }

    private var yoursSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            HStack {
                Text("Your topics").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                Spacer()
                Button("Add topic") { editor = .init(seed: nil) }.buttonStyle(.whimbrelSecondary)
            }
            if seeds.isEmpty {
                Text("No topics yet. Add one, or accept a suggestion below.")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            ForEach(seeds) { seed in
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(seed.name).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelTextPrimary)
                            let detail = ([seed.kind.map(Self.kindLabel)].compactMap { $0 } + [seed.aliases.joined(separator: ", ")])
                                .filter { !$0.isEmpty }.joined(separator: " · ")
                            if !detail.isEmpty {
                                Text(detail).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                            }
                        }
                        Spacer(minLength: Spacing.sm)
                        Text("\(seed.itemCount) items").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                        Button("Edit") { editor = .init(seed: seed) }.buttonStyle(.whimbrelSecondary)
                        if seeds.count > 1 {
                            Button("Merge into…") {
                                mergingKey = seed.key
                                deletingKey = nil
                                mergeInto = seeds.first { $0.key != seed.key }?.key ?? ""
                            }.buttonStyle(.whimbrelSecondary)
                        }
                        Button("Delete") { deletingKey = seed.key; mergingKey = nil }.buttonStyle(.whimbrelSecondary)
                    }
                    if deletingKey == seed.key {
                        HStack(spacing: Spacing.sm) {
                            Text("Delete \(seed.name)? Items stay in your timeline.")
                                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
                            Button("Delete") {
                                deletingKey = nil
                                run({ $0.topicSeedDelete(key: seed.key) }, failure: "Could not delete that topic.")
                            }.buttonStyle(.whimbrelPrimary)
                            Button("Cancel") { deletingKey = nil }.buttonStyle(.whimbrelSecondary)
                        }
                    }
                    if mergingKey == seed.key {
                        HStack(spacing: Spacing.sm) {
                            Text("Merge \(seed.name) into").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                            Picker("Merge into", selection: $mergeInto) {
                                ForEach(seeds.filter { $0.key != seed.key }) { Text($0.name).tag($0.key) }
                            }.labelsHidden().frame(maxWidth: 200)
                            Button("Merge") {
                                let keep = mergeInto
                                mergingKey = nil
                                run({ $0.topicSeedMerge(keep: keep, absorb: seed.key) }, failure: "Could not merge those topics.")
                            }.buttonStyle(.whimbrelPrimary).disabled(mergeInto.isEmpty)
                            Button("Cancel") { mergingKey = nil }.buttonStyle(.whimbrelSecondary)
                        }
                    }
                }
            }
        }
    }

    private var suggestedSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Suggested").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            if suggestions.isEmpty {
                Text("When a theme shows up across several of your meetings, emails and notes, Whimbrel suggests it here. Nothing is added until you press Add.")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            ForEach(suggestions) { s in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                    TextField("Name", text: Binding(get: { suggestionNames[s.topicId] ?? s.name },
                                                    set: { suggestionNames[s.topicId] = $0 }))
                        .textFieldStyle(.roundedBorder)
                    Text("\(s.itemCount) items").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    Button("Add") {
                        let name = (suggestionNames[s.topicId] ?? s.name).trimmingCharacters(in: .whitespaces)
                        run({ $0.topicSuggestionAccept(topicId: s.topicId, name: name) }, failure: "Could not add that topic.")
                    }
                    .buttonStyle(.whimbrelPrimary)
                    .disabled((suggestionNames[s.topicId] ?? s.name).trimmingCharacters(in: .whitespaces).isEmpty)
                    Button("Dismiss") {
                        run({ $0.topicSuggestionDismiss(topicId: s.topicId) }, failure: "Could not dismiss that suggestion.")
                    }.buttonStyle(.whimbrelSecondary)
                }
            }
        }
    }

    private var namesSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Names I go by").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Whimbrel uses these to tell steps you owe from steps owed to you.")
                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            ForEach(names, id: \.self) { name in
                HStack(spacing: Spacing.sm) {
                    Text(name).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelTextPrimary)
                    Spacer(minLength: Spacing.sm)
                    Button("Remove") { saveNames(names.filter { $0 != name }) }.buttonStyle(.whimbrelSecondary)
                }
            }
            HStack(spacing: Spacing.sm) {
                TextField("Add a name", text: $newName)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(addNewName)
                Button("Add", action: addNewName)
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            let chips = nameChips
            if !chips.isEmpty {
                HStack(spacing: Spacing.sm) {
                    ForEach(chips, id: \.self) { chip in
                        Button("+ \(chip)") { saveNames(names + [chip]) }.buttonStyle(.whimbrelSecondary)
                    }
                }
            }
        }
    }

    /// Core's suggestions plus the Profile name, minus names already listed (ignoring case);
    /// at most 4 so the row fits.
    private var nameChips: [String] {
        var seen = Set(names.map { $0.lowercased() })
        return (suggestedNames + [appState.profile.name])
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty && seen.insert($0.lowercased()).inserted }
            .prefix(4).map { $0 }
    }

    private func addNewName() {
        let name = newName.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        newName = ""
        saveNames(names + [name])
    }

    /// Saves the list trimmed, without blanks or case-insensitive repeats.
    private func saveNames(_ list: [String]) {
        var seen = Set<String>()
        let clean = list.map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty && seen.insert($0.lowercased()).inserted }
        names = clean
        run({ $0.topicNamesSet(clean) }, failure: "Could not save your names.")
    }

    private var stepsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Toggle("Find steps with TypeSafe", isOn: Binding(
                get: { appState.topicSteps.enabled },
                set: { appState.setTopicStepsEnabled($0) }
            ))
            .tint(Color.whimbrelAccentPrimary)
            .disabled(!appState.replyPriority.hasKey)
            Text("Sends short excerpts of topic emails, meeting action items and Teams screens to TypeSafe to find steps you owe and are owed. Off: your local model does this.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if appState.topicSteps.awaitingStart {
                if appState.topicSteps.estimateFailed {
                    HStack {
                        Text("Couldn't get an estimate. Try again.")
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelError)
                        Button("Try again") { appState.refreshTopicStepsEstimate() }
                    }
                } else if let e = appState.topicSteps.estimate {
                    if e.indexing || e.sources == 0 {
                        Text("Still sorting your items. The estimate will appear when that's done.")
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    } else {
                        HStack {
                            Text("About \(e.sources) items, about \(e.usd, format: .currency(code: "USD")). Start?")
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                            Button("Start") { appState.startTopicSteps() }
                        }
                    }
                }
            }
            if appState.topicSteps.startFailed {
                Text("Couldn't start finding steps. Try again.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }
            if appState.topicSteps.keyRejected {
                Text("TypeSafe rejected this key. Check it and save it again.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }
            if appState.topicSteps.overBudget {
                Text("The budget for finding steps is used up. Turn the switch off and on again to see a new estimate.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }
            if appState.topicStepsNoModel {
                Text("Finding steps needs a local model (Settings → AI).")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    /// Reads the lists off the main thread and publishes them on main.
    private func reload() {
        guard let store = appState.coreStore else { return }
        appState.refreshTopicStepsEstimate()
        CoreMemoryStore.accessQueue.async {
            let s = store.topicSeeds()
            let g = store.topicSuggestions()
            let n = store.topicNames()
            DispatchQueue.main.async {
                if let s { seeds = s }
                if let g { suggestions = g }
                if let n { names = n.names; suggestedNames = n.suggested }
            }
        }
    }

    static func kindLabel(_ kind: String) -> String {
        switch kind {
        case "customer": String(localized: "Customer")
        case "product": String(localized: "Product")
        case "project": String(localized: "Project")
        case "subject": String(localized: "Subject")
        default: kind.capitalized
        }
    }

    /// Runs a store write off the main thread, then reloads; shows `failure` if it returns false.
    private func run(_ work: @escaping (CoreMemoryStore) -> Bool, failure: LocalizedStringResource) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let ok = work(store)
            DispatchQueue.main.async {
                errorText = ok ? nil : String(localized: failure)
                reload()
            }
        }
    }
}

struct TopicEditorTarget: Identifiable {
    let seed: TopicSeed?
    var id: String { seed?.key ?? "new" }
}

private struct TopicEditorSheet: View {
    let target: TopicEditorTarget
    let onSave: (String, [String], String?) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var aliases: String
    @State private var kind: String?

    init(target: TopicEditorTarget, onSave: @escaping (String, [String], String?) -> Void) {
        self.target = target
        self.onSave = onSave
        _name = State(initialValue: target.seed?.name ?? "")
        _aliases = State(initialValue: target.seed?.aliases.joined(separator: ", ") ?? "")
        _kind = State(initialValue: target.seed?.kind)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            (target.seed == nil ? Text("Add topic") : Text("Edit topic")).font(.whimbrelDisplay())
            TextField("Name", text: $name).textFieldStyle(.roundedBorder)
            TextField("Aliases, separated by commas", text: $aliases).textFieldStyle(.roundedBorder)
            Picker("Kind", selection: $kind) {
                Text("None").tag(String?.none)
                Text("Customer").tag(String?.some("customer"))
                Text("Product").tag(String?.some("product"))
                Text("Project").tag(String?.some("project"))
                Text("Subject").tag(String?.some("subject"))
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.buttonStyle(.whimbrelSecondary)
                Button("Save") {
                    let list = aliases.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
                    onSave(name.trimmingCharacters(in: .whitespaces), list, kind)
                    dismiss()
                }
                .buttonStyle(.whimbrelPrimary)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(Spacing.lg)
        .frame(width: 420)
    }
}
