import AppKit
import Combine
import OSLog

/// The skin maker's reply (core/llm `SkinTurnReply`, camelCase). `skin` and each palette are the
/// model's JSON objects, re-serialized so `SkinLoader`/`ThemeLoader` can validate them.
struct SkinTurnReplyDTO {
    enum Intent: String, Decodable { case new, refine, clarify, decline }
    struct Note: Decodable, Equatable { var step: String; var text: String }

    var intent: Intent
    var reply = ""
    var notes: [Note] = []
    var skin: Data?
    var palettes: [Data] = []
    var imageQueries: [String] = []
    var imagePrompts: [String] = []
}

extension SkinTurnReplyDTO: Decodable {
    private enum CodingKeys: String, CodingKey { case intent, reply, notes, skin, palettes, imageQueries, imagePrompts }

    /// Any JSON value, as `JSONSerialization` would read it.
    private struct AnyJSON: Decodable {
        let value: Any
        init(from decoder: Decoder) throws {
            let c = try decoder.singleValueContainer()
            if c.decodeNil() { value = NSNull() }
            else if let b = try? c.decode(Bool.self) { value = b }
            else if let n = try? c.decode(Double.self) { value = n }
            else if let s = try? c.decode(String.self) { value = s }
            else if let a = try? c.decode([AnyJSON].self) { value = a.map(\.value) }
            else { value = try c.decode([String: AnyJSON].self).mapValues(\.value) }
        }
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func object(_ json: AnyJSON?) -> Data? {
            guard let dict = json?.value as? [String: Any] else { return nil }
            return try? JSONSerialization.data(withJSONObject: dict, options: [.sortedKeys])
        }
        intent = try c.decode(Intent.self, forKey: .intent)
        reply = try c.decodeIfPresent(String.self, forKey: .reply) ?? ""
        notes = try c.decodeIfPresent([Note].self, forKey: .notes) ?? []
        skin = object(try c.decodeIfPresent(AnyJSON.self, forKey: .skin))
        palettes = (try c.decodeIfPresent([AnyJSON].self, forKey: .palettes) ?? []).compactMap(object)
        imageQueries = try c.decodeIfPresent([String].self, forKey: .imageQueries) ?? []
        imagePrompts = try c.decodeIfPresent([String].self, forKey: .imagePrompts) ?? []
    }
}

/// A found or generated image waiting for approval (core/llm `ImageCandidate`). A generated one
/// has a `generated:` url and its bytes are held by the session.
struct ImageCandidateDTO: Decodable, Identifiable, Equatable {
    var url: String
    var thumbnailUrl: String
    var title: String
    /// "web" or "generated".
    var source: String
    var id: String { url }
}

/// What the skin chat may use right now. The chat follows `TodayComposer.askAvailable` on the
/// default provider (so local-only mode blocks a hosted one); web search and image generation
/// are hosted, so local-only mode turns them off too.
struct SkinChatAccess: Equatable {
    var provider: RoutingSettings.Provider
    var model: String
    var localOnly: Bool
    var hasKey: Bool
    var hasSearchKey: Bool
    var hasImageKey: Bool

    var chatAvailable: Bool { TodayComposer.askAvailable(provider: provider, localOnly: localOnly, hasKey: hasKey) }
    var canSearch: Bool { hasSearchKey && !localOnly }
    var canGenerate: Bool { hasImageKey && !localOnly }
}

extension SkinChatAccess {
    @MainActor init(routing: RoutingSettings) {
        let selection = routing.defaultSelection()
        provider = selection.provider
        model = selection.provider == .ollama ? selection.ollamaModel : selection.hostedModel
        localOnly = routing.localOnlyMode
        switch selection.provider {
        case .ollama: hasKey = true
        case .anthropic: hasKey = routing.hasAnthropicKey
        case .openai: hasKey = routing.hasOpenAIKey
        case .google: hasKey = routing.hasGoogleKey
        }
        hasSearchKey = KeychainService.exists(for: .imageSearch)
        hasImageKey = routing.hasOpenAIKey || routing.hasGoogleKey
    }
}

/// The skin maker's calls. All may be slow; the session awaits them off the main actor.
protocol SkinTurnBackend {
    func turn(_ input: Data, provider: RoutingSettings.Provider, model: String) async throws -> SkinTurnReplyDTO
    func search(_ query: String) async throws -> [ImageCandidateDTO]
    func generate(_ prompt: String) async throws -> Data
    /// Downloads an image through the core's guarded fetch (https, public addresses only).
    func fetch(_ url: String) async throws -> Data
}

/// The core FFI, with keys from the Keychain and the spend caps from `RoutingSettings`.
/// Errors are `CoreLLMError` (`.spendCap` when the daily cap stops a call).
struct CoreSkinTurnBackend: SkinTurnBackend {
    let routing: RoutingSettings
    var dbPath = SpikePaths.database.path

    /// The FFI calls block for the whole network call (up to 120 s), so they run here, never on Swift's cooperative
    /// pool (one thread per core: a strip of thumbnails would stall every other task). Concurrent, so thumbnails
    /// don't queue behind one another. Like `CoreMemoryStore.slowLaneQueue` for the store.
    static let queue = DispatchQueue(label: "com.mpointer.kestrel.skin-maker", qos: .userInitiated, attributes: .concurrent)

    /// `work` on `queue`; its result (or error) once it finishes. A stopped run still waits for it and ignores it.
    static func offPool<T>(_ work: @escaping @Sendable () -> Result<T, CoreLLMError>) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { continuation.resume(with: work()) }
        }
    }

    private func config(_ provider: RoutingSettings.Provider? = nil, model: String? = nil) async -> CoreRoutingConfig {
        await MainActor.run {
            let selection = routing.defaultSelection()
            return routing.routingConfig(preferredProvider: provider ?? selection.provider, preferredModel: model ?? selection.hostedModel)
        }
    }

    func turn(_ input: Data, provider: RoutingSettings.Provider, model: String) async throws -> SkinTurnReplyDTO {
        let config = await config(provider, model: model)
        let key: String?
        switch provider {
        case .ollama: key = nil
        case .anthropic: key = config.anthropicKey
        case .openai: key = config.openaiKey
        case .google: key = config.googleKey
        }
        let dbPath = dbPath
        return try await Self.offPool {
            CoreSkinMaker.turn(dbPath: dbPath, provider: provider.rawValue, model: model, apiKey: key, input: input, routing: config)
        }
    }

    func search(_ query: String) async throws -> [ImageCandidateDTO] {
        let config = await config()
        let dbPath = dbPath
        return try await Self.offPool {
            guard let key = KeychainService.load(for: .imageSearch) else { return .failure(.providerFailure) }
            return CoreSkinMaker.searchImages(dbPath: dbPath, apiKey: key, query: query, routing: config)
        }
    }

    func generate(_ prompt: String) async throws -> Data {
        let config = await config()
        let preferGoogle = config.preferredProvider == RoutingSettings.Provider.google.rawValue && config.googleKey != nil
        let pick = preferGoogle ? ("google", config.googleKey) : config.openaiKey != nil ? ("openai", config.openaiKey) : ("google", config.googleKey)
        guard let key = pick.1 else { throw CoreLLMError.providerFailure }
        let dbPath = dbPath, provider = pick.0
        return try await Self.offPool {
            CoreSkinMaker.generateImage(dbPath: dbPath, provider: provider, apiKey: key, prompt: prompt, routing: config)
        }
    }

    func fetch(_ url: String) async throws -> Data {
        try await Self.offPool { CoreSkinMaker.fetchImage(url) }
    }
}

/// "Make a skin by talking to Whimbrel": one chat per skin. A turn sends the request, the skin's
/// chat and manifest to the model, then saves the reply as a user skin (a new one, a fork of a
/// built-in, or a new version of yours) and switches to it; found and generated images wait in
/// `candidates` until approved. See docs/superpowers/specs/2026-10-21-skins-r2-conversation-design.md.
@MainActor
final class SkinChatSession: ObservableObject {
    @Published private(set) var status: [String] = []
    /// The chat of the selected skin.
    @Published private(set) var turns: [SkinChatTurn] = []
    @Published private(set) var candidates: [ImageCandidateDTO] = []
    @Published private(set) var isRunning = false
    /// The chat field's text, kept here so typing survives the window rebuild a skin switch causes.
    @Published var draft = ""

    static let searchLimit = 6
    static let spendCapLine = "Today's AI spending limit is reached."

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")
    private let skinStore: SkinStore
    private let userSkins: UserSkinStore
    private let backend: SkinTurnBackend
    private let access: () -> SkinChatAccess
    /// The skin `turns` belong to, and the one `candidates` are for.
    private var chatSkinId: String
    private var candidateSkinId: String?
    /// Bumped by every send and by `stop()`; a run whose number is stale applies nothing.
    private var run = 0
    /// The latest turn or image action, so callers (tests) can await it.
    private(set) var current: Task<Void, Never>?
    private var generated: [String: Data] = [:]
    private var thumbnails: [String: NSImage] = [:]
    private var selectionWatch: AnyCancellable?

    /// `access` nil reads it from `routing` and the Keychain each time.
    init(skinStore: SkinStore, userSkins: UserSkinStore, backend: SkinTurnBackend, routing: RoutingSettings,
         access: (() -> SkinChatAccess)? = nil) {
        self.skinStore = skinStore
        self.userSkins = userSkins
        self.backend = backend
        self.access = access ?? { SkinChatAccess(routing: routing) }
        chatSkinId = skinStore.selected.id
        turns = userSkins.chat(for: chatSkinId)
        selectionWatch = skinStore.$revision.receive(on: DispatchQueue.main).sink { [weak self] _ in
            MainActor.assumeIsolated { self?.followSelection() }
        }
    }

    var available: Bool { access().chatAvailable }

    /// Shows the newly selected skin's chat (and drops the strip, which belonged to the old one).
    private func followSelection() {
        let id = skinStore.selected.id
        guard id != chatSkinId else { return }
        chatSkinId = id
        turns = userSkins.chat(for: id)
        candidates = []
        candidateSkinId = nil
        generated = [:]
        thumbnails = [:]
        if !isRunning { status = [] }
    }

    // MARK: turns

    func send(_ text: String) {
        let request = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !request.isEmpty, !isRunning else { return }
        let access = self.access()
        let origin = skinStore.selected
        guard access.chatAvailable else {
            post("Set up an AI provider in Settings to make skins.", to: origin.id, store: false)
            return
        }
        run += 1
        let myRun = run
        isRunning = true
        let history = turns.map { ["role": $0.role, "text": $0.text] }
        let userTurn = SkinChatTurn(role: "user", text: request, date: Date())
        turns.append(userTurn)
        status = ["Reading your request…"]
        if access.provider != .ollama { status.append("Sent to \(access.provider.displayName): your request, this skin's chat, its design and the list of allowed values") }
        let sent = manifestObject(for: origin)
        let input: [String: Any] = [
            "request": request, "history": history, "catalogue": SkinCatalogue.object(),
            "currentManifest": sent ?? NSNull(),
        ]
        let data = (try? JSONSerialization.data(withJSONObject: input)) ?? Data()
        let sentAssets = Set(Self.imagery(of: sent).compactMap(Self.asset))
        current = Task { await perform(data, userTurn: userTurn, origin: origin, sentAssets: sentAssets, access: access, run: myRun) }
    }

    /// Ends the run; whatever it returns later is ignored.
    func stop() {
        guard isRunning else { return }
        run += 1
        isRunning = false
        status.append("Stopped.")
    }

    /// `sentAssets`: the images in the manifest the model was given, so one approved meanwhile is kept.
    private func perform(_ input: Data, userTurn: SkinChatTurn, origin: Skin, sentAssets: Set<String>, access: SkinChatAccess, run myRun: Int) async {
        defer { if run == myRun { isRunning = false } }
        let reply: SkinTurnReplyDTO
        do {
            reply = try await backend.turn(input, provider: access.provider, model: access.model)
        } catch {
            guard run == myRun else { return }
            Self.log.error("skin turn failed: \(String(describing: error), privacy: .public)")
            post(Self.isSpendCap(error) ? Self.spendCapLine : "Whimbrel couldn't make that skin, so nothing changed. Try again or say it another way.",
                 to: origin.id, store: false)
            return
        }
        guard run == myRun else { return }
        status += reply.notes.map(\.text)

        guard reply.intent == .new || reply.intent == .refine else {
            // A question back or a decline: chat only.
            let saved = userSkins.manifest(for: origin.id) != nil
            if saved { try? userSkins.appendChat(userTurn, to: origin.id) }
            post(reply.reply.isEmpty ? "Could you say a bit more about the skin you want?" : reply.reply, to: origin.id, store: saved)
            return
        }
        guard let (target, lines) = saveSkin(reply, origin: origin, sentAssets: sentAssets) else {
            post("Whimbrel couldn't make a usable skin from that, so nothing changed. Try again or say it another way.", to: origin.id, store: false)
            return
        }
        try? userSkins.appendChat(userTurn, to: target)
        for line in [reply.reply.isEmpty ? "Here's your skin." : reply.reply] + lines {
            try? userSkins.appendChat(SkinChatTurn(role: "whimbrel", text: line, date: Date()), to: target)
        }
        let wasSelected = skinStore.selected.id == origin.id
        let paletteId = skinStore.selectedPaletteId
        reloadAndMarkScroll()
        if wasSelected {
            skinStore.select(skinId: target, paletteId: target == origin.id ? paletteId : nil)
            chatSkinId = target
            turns = userSkins.chat(for: target)
        } else if chatSkinId == target {
            turns = userSkins.chat(for: target)
        }
        await findImages(reply, request: userTurn.text, for: target, access: access, run: myRun)
    }

    /// Searches for a new skin (or when the request mentions images) and generates only when the
    /// request asks for an image, whatever the model returned (R4).
    private func findImages(_ reply: SkinTurnReplyDTO, request: String, for skinId: String, access: SkinChatAccess, run myRun: Int) async {
        let mentionsImages = Self.mentionsImages(request)
        var found = 0
        if access.canSearch && (reply.intent == .new || mentionsImages) {
            for query in reply.imageQueries where found < Self.searchLimit {
                guard run == myRun else { return }
                status.append("Searching the web for '\(query)'…")
                do {
                    let results = try await backend.search(query)
                    guard run == myRun else { return }
                    let fresh = results.filter { !candidates.contains($0) }.prefix(Self.searchLimit - found)
                    found += fresh.count
                    show(Array(fresh), for: skinId)
                } catch {
                    guard run == myRun else { return }
                    if Self.isSpendCap(error) { post(Self.spendCapLine, to: skinId, store: true); return }
                    Self.log.error("image search failed: \(String(describing: error), privacy: .public)")
                }
            }
        }
        guard access.canGenerate && mentionsImages else { return }
        for prompt in reply.imagePrompts {
            guard run == myRun else { return }
            status.append("Making an image: '\(prompt)'…")
            do {
                let data = try await backend.generate(prompt)
                guard run == myRun else { return }
                let url = "generated:\(UUID().uuidString)"
                generated[url] = data
                show([ImageCandidateDTO(url: url, thumbnailUrl: url, title: prompt, source: "generated")], for: skinId)
            } catch {
                guard run == myRun else { return }
                if Self.isSpendCap(error) { post(Self.spendCapLine, to: skinId, store: true); return }
                Self.log.error("image generation failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    private func show(_ found: [ImageCandidateDTO], for skinId: String) {
        guard !found.isEmpty, chatSkinId == skinId else { return }
        if candidateSkinId != skinId { candidates = [] }
        candidateSkinId = skinId
        candidates += found
    }

    /// The image words of R4, at a word start ("photos", "backgrounds" count).
    static func mentionsImages(_ text: String) -> Bool {
        text.range(of: #"\b(image|picture|photo|banner|logo|art|background)"#, options: [.regularExpression, .caseInsensitive]) != nil
    }

    private static func isSpendCap(_ error: Error) -> Bool {
        if case .spendCap? = error as? CoreLLMError { return true }
        return false
    }

    // MARK: saving a reply

    /// The reply's manifest saved as a user skin: palettes repaired and given `<id>-p<n>` ids (a
    /// refine keeps the ids of its own palettes the reply returns or still lists, so the selected
    /// colours stay put), imagery limited to approved files (plus any approved while the turn ran).
    /// Returns its id and the chat lines explaining any repair or fallback; nil when no usable
    /// manifest survives.
    private func saveSkin(_ reply: SkinTurnReplyDTO, origin: Skin, sentAssets: Set<String>) -> (String, [String])? {
        guard let data = reply.skin, var manifest = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return nil }
        // Only what the loader reads is kept, so nothing else the model wrote is stored, re-sent or exported.
        manifest = manifest.filter { SkinLoader.topLevelKeys.contains($0.key) }
        let target: String
        let ownSkin = origin.id.hasPrefix("user-") && userSkins.manifest(for: origin.id) != nil
        // Capped first, so a name of only spaces counts as missing.
        let replyName = UserSkinStore.skinName(manifest["name"] as? String ?? "")
        if reply.intent == .refine && ownSkin {
            target = origin.id
            manifest["name"] = replyName.isEmpty ? origin.name : replyName
        } else if reply.intent == .refine {
            target = userSkins.newSkinId(slug: origin.id)
            manifest["name"] = UserSkinStore.skinName(origin.name, suffix: " (yours)")
        } else {
            let name = replyName.isEmpty ? "My skin" : replyName
            manifest["name"] = name
            target = userSkins.newSkinId(slug: name)
        }
        manifest["id"] = target
        let refiningOwn = reply.intent == .refine && ownSkin

        var lines: [String] = []
        let existing = refiningOwn ? userSkins.palettes(for: target) : []
        var palettes: [(id: String, data: Data)] = []
        var renamed: [String: String] = [:]
        var next = 1
        func taken(_ id: String) -> Bool { existing.contains { $0.id == id } || palettes.contains { $0.id == id } }
        for raw in reply.palettes {
            let original = ((try? JSONSerialization.jsonObject(with: raw)) as? [String: Any])?["id"] as? String
            let id: String
            if let original, existing.contains(where: { $0.id == original }), !palettes.contains(where: { $0.id == original }) {
                id = original
            } else {
                while taken("\(target)-p\(next)") { next += 1 }
                id = "\(target)-p\(next)"
            }
            guard let palette = Self.repairedPalette(raw, id: id) else { continue }
            palettes.append((id, palette.data))
            if let original, renamed[original] == nil { renamed[original] = id }
            if !palette.changed.isEmpty {
                let words = palette.changed.map(Self.words).joined(separator: ", ")
                lines.append("I adjusted \(words) in “\(palette.name)” so text stays readable.")
            }
        }
        let allRejected = !reply.palettes.isEmpty && palettes.isEmpty
        var paletteIds: [String] = []
        for ref in ((manifest["palettes"] as? [Any]) ?? []).compactMap({ $0 as? String }) {
            let id: String?
            if let new = renamed[ref] {
                id = new
            } else if let kept = existing.first(where: { $0.id == ref }) {
                if !palettes.contains(where: { $0.id == kept.id }) { palettes.append(kept) }
                id = kept.id
            } else {
                id = nil
            }
            if let id, !paletteIds.contains(id) { paletteIds.append(id) }
        }
        if paletteIds.isEmpty { paletteIds = palettes.map(\.id) }
        if palettes.isEmpty && reply.intent == .refine {
            // No usable new colours: keep the skin's own (a fork keeps the built-in's, which are installed anyway).
            palettes = existing
            paletteIds = ownSkin ? existing.map(\.id) : origin.paletteIds
            if allRejected && !paletteIds.isEmpty {
                lines.append("The new colours didn't pass Whimbrel's checks, so this skin keeps its current colours.")
            }
        }
        if paletteIds.isEmpty {
            paletteIds = [ThemeDefaults.theme.id]
            lines.append("I couldn't make a usable palette, so this skin uses Whimbrel's colours for now.")
        }
        // Palettes not repaired above (a fork's built-ins, Whimbrel's own, your earlier ones) must still suit this
        // skin's archetype: one that doesn't is replaced by a repaired copy of its own.
        let archetype = ((manifest["today"] as? [String: Any])?["archetype"] as? String).flatMap(SkinArchetype.init) ?? SkinDefaults.whimbrel.archetype
        for (index, ref) in paletteIds.enumerated() {
            let own = palettes.firstIndex { $0.id == ref }
            guard let theme = own.flatMap({ ThemeLoader.parse(palettes[$0].data) }) ?? skinStore.palette(id: ref) else { continue }
            if own == nil { while taken("\(target)-p\(next)") { next += 1 } }
            let id = own == nil ? "\(target)-p\(next)" : ref
            guard let fixed = Self.archetypeRepair(theme, id: id, archetype: archetype) else { continue }
            if let own { palettes[own] = (id, fixed.data) } else { palettes.append((id, fixed.data)) }
            paletteIds[index] = id
            lines.append("I adjusted \(fixed.changed.map(Self.words).joined(separator: ", ")) in “\(fixed.name)” so text stays readable.")
        }
        manifest["palettes"] = paletteIds

        // Only images already approved into this skin's folder may be placed; one approved while
        // this turn ran (so the model never saw it) is carried over from the saved manifest.
        var imagery = Self.imagery(of: manifest["decoration"].map { ["decoration": $0] })
        if refiningOwn {
            let replied = Set(imagery.compactMap(Self.asset))
            let saved = userSkins.manifest(for: target).flatMap { try? JSONSerialization.jsonObject(with: $0) }
            let approvedMeanwhile = Self.imagery(of: saved).filter { Self.asset($0).map { !sentAssets.contains($0) && !replied.contains($0) } ?? false }
            for entry in approvedMeanwhile { imagery = Self.placing(entry, in: imagery) }
        }
        imagery = imagery.filter { item in
            guard let asset = Self.asset(item), asset != ".", asset != "..", !asset.contains("/") else { return false }
            var isDirectory: ObjCBool = false
            return FileManager.default.fileExists(atPath: userSkins.imageURL(asset: asset, skinId: target).path, isDirectory: &isDirectory)
                && !isDirectory.boolValue
        }
        if manifest["decoration"] != nil || !imagery.isEmpty {
            var decoration = manifest["decoration"] as? [String: Any] ?? [:]
            decoration["imagery"] = imagery
            manifest["decoration"] = decoration
        }

        guard let manifestData = try? JSONSerialization.data(withJSONObject: manifest, options: [.prettyPrinted, .sortedKeys]),
              SkinLoader.parse(manifestData, base: SkinDefaults.whimbrel) != nil else { return nil }
        do {
            _ = try userSkins.save(manifest: manifestData, palettes: palettes, for: target)
        } catch {
            Self.log.error("skin save failed: \(String(describing: error), privacy: .public)")
            return nil
        }
        return (target, lines)
    }

    /// The palette repair the session uses; tests swap in one that gives up.
    nonisolated(unsafe) static var repairPalette: (ThemePalette) -> (palette: ThemePalette, changed: [ThemeToken]) = { PaletteRepair.repair($0) }

    /// The palette with id `id`, each variant (gaps filled from Default, as `ThemeLoader` does) repaired
    /// to pass every archetype's pairs (`ArchetypeContrast`, since any skin may take it); nil when a variant
    /// still fails (so the visible fallback runs, not `ThemeLoader`'s silent swap to Default) or
    /// `ThemeLoader` would not load it.
    nonisolated static func repairedPalette(_ raw: Data, id: String) -> (data: Data, originalId: String?, name: String, changed: [ThemeToken])? {
        guard var object = (try? JSONSerialization.jsonObject(with: raw)) as? [String: Any] else { return nil }
        let originalId = object["id"] as? String
        object["id"] = id
        let name = (object["name"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? "Colours"
        object["name"] = name
        var changed: [ThemeToken] = []
        for (key, base) in [("light", ThemeDefaults.theme.light), ("dark", ThemeDefaults.theme.dark)] {
            guard var variant = object[key] as? [String: Any], let parsed = ThemeLoader.variant(variant) else { continue }
            let (fixed, moved) = Self.repairPalette(base.merging(parsed) { $1 })
            guard ArchetypeContrast.passes(fixed) else { return nil }
            for token in moved {
                variant[token.rawValue] = String(format: "#%06x", fixed[token] ?? 0)
                if !changed.contains(token) { changed.append(token) }
            }
            object[key] = variant
        }
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys]),
              ThemeLoader.parse(data) != nil else { return nil }
        return (data, originalId, name, changed)
    }

    /// nil when `theme` passes `archetype`'s pairs (`ArchetypeContrast`) in both appearances, or when it can't be
    /// repaired (it is then kept as it is); otherwise a copy with id `id` repaired by `repairedPalette`. Used for
    /// palettes that skipped repair (kept or fallback ones, imported ones).
    nonisolated static func archetypeRepair(_ theme: Theme, id: String, archetype: SkinArchetype) -> (data: Data, name: String, changed: [ThemeToken])? {
        let pairs = ArchetypeContrast.pairs(for: archetype)
        guard !ArchetypeContrast.passes(theme.light, pairs) || !ArchetypeContrast.passes(theme.dark, pairs) else { return nil }
        func hex(_ p: ThemePalette) -> [String: String] { Dictionary(uniqueKeysWithValues: p.map { ($0.key.rawValue, String(format: "#%06x", $0.value)) }) }
        let raw: [String: Any] = ["id": theme.id, "name": theme.name, "light": hex(theme.light), "dark": hex(theme.dark)]
        guard let data = try? JSONSerialization.data(withJSONObject: raw), let fixed = repairedPalette(data, id: id) else { return nil }
        return (fixed.data, fixed.name, fixed.changed)
    }

    /// The `decoration.imagery` entries of a manifest object.
    private static func imagery(of manifest: Any?) -> [Any] {
        (((manifest as? [String: Any])?["decoration"] as? [String: Any])?["imagery"] as? [Any]) ?? []
    }

    private static func asset(_ entry: Any) -> String? { (entry as? [String: Any])?["asset"] as? String }

    /// `textSecondary` → "text secondary".
    private static func words(_ token: ThemeToken) -> String {
        token.rawValue.reduce(into: "") { out, c in out += c.isUppercase ? " \(c.lowercased())" : String(c) }
    }

    /// The skin's manifest as sent to the model: a user skin's `skin.json`, a built-in's bundled file.
    private func manifestObject(for skin: Skin) -> Any? {
        let data = userSkins.manifest(for: skin.id)
            ?? SkinStore.bundledSkinsDirectory.flatMap { try? Data(contentsOf: $0.appendingPathComponent("\(skin.id).json")) }
        return data.flatMap { try? JSONSerialization.jsonObject(with: $0) }
    }

    // MARK: undo

    func undo() {
        guard !isRunning else { return }
        let id = skinStore.selected.id
        guard userSkins.manifest(for: id) != nil else {
            post("Built-in skins never change, so there's nothing to undo.", to: id, store: false)
            return
        }
        do {
            if try userSkins.undo(skinId: id) {
                reloadAndMarkScroll()
                post("Undone: this skin is back to its previous version.", to: id, store: true)
            } else {
                post("There's no earlier version to go back to.", to: id, store: false)
            }
        } catch {
            Self.log.error("skin undo failed: \(String(describing: error), privacy: .public)")
            post("Whimbrel couldn't undo that.", to: id, store: false)
        }
    }

    // MARK: images

    /// The candidate's thumbnail, fetched only through the backend's guarded fetch and cached.
    /// A thumbnail that will not load drops the candidate from the strip; in local-only mode nothing is fetched.
    func thumbnail(for candidate: ImageCandidateDTO) async -> NSImage? {
        if let cached = thumbnails[candidate.url] { return cached }
        let data: Data?
        if let bytes = generated[candidate.url] {
            data = bytes
        } else {
            // Local-only mode may have been turned on since the image was found.
            guard !access().localOnly else { return nil }
            data = try? await backend.fetch(candidate.thumbnailUrl)
        }
        guard let image = data.flatMap(NSImage.init(data:)) else {
            candidates.removeAll { $0 == candidate }
            return nil
        }
        thumbnails[candidate.url] = image
        return image
    }

    /// Downloads the image (generated ones are already here), stores it in the skin's folder and
    /// places it as the header image (replacing any earlier one) in a new version. Nothing is downloaded in local-only mode.
    func approve(_ candidate: ImageCandidateDTO) {
        guard let skinId = candidateSkinId, candidates.contains(candidate) else { return }
        guard generated[candidate.url] != nil || !access().localOnly else {
            post("Local-only mode is on, so Whimbrel didn't download that image.", to: skinId, store: false)
            return
        }
        candidates.removeAll { $0 == candidate }
        let bytes = generated.removeValue(forKey: candidate.url)
        thumbnails[candidate.url] = nil
        current = Task {
            do {
                let data: Data
                if let bytes { data = bytes } else { data = try await backend.fetch(candidate.url) }
                try place(data, name: bytes == nil ? candidate.url : "generated.png", in: skinId)
            } catch {
                Self.log.error("image approve failed: \(String(describing: error), privacy: .public)")
                post("That image couldn't be downloaded, so it was left out.", to: skinId, store: true)
            }
        }
    }

    /// Forgets the candidate; nothing was downloaded, so nothing is on disk.
    func reject(_ candidate: ImageCandidateDTO) {
        candidates.removeAll { $0 == candidate }
        generated[candidate.url] = nil
        thumbnails[candidate.url] = nil
    }

    /// A file the user picked (approval is implicit) placed in the selected skin, which must be one of yours.
    func addOwnImage(_ url: URL) {
        let skinId = skinStore.selected.id
        guard userSkins.manifest(for: skinId) != nil else {
            post("Ask for a change first, so Whimbrel makes your own copy of this skin, then add the image.", to: skinId, store: false)
            return
        }
        current = Task {
            do {
                let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                guard size <= Self.imageByteLimit else {
                    post("That image is over 10 MB, so Whimbrel left it out.", to: skinId, store: true)
                    return
                }
                let data = try await Task.detached { try Data(contentsOf: url) }.value
                guard Self.isUsableImage(data) else {
                    post("That file isn't an image Whimbrel can use.", to: skinId, store: true)
                    return
                }
                // A neutral name by type: the file's own name (say, a person's) would reach the provider in the manifest.
                try place(data, name: "own.\(Self.fileExtension(of: data))", in: skinId)
            } catch {
                Self.log.error("own image failed: \(String(describing: error), privacy: .public)")
                post("Whimbrel couldn't read that file.", to: skinId, store: true)
            }
        }
    }

    /// The web fetch's limit (core `whimbrel_skin_image_fetch`), applied to your own files too.
    nonisolated static let imageByteLimit = 10 * 1024 * 1024

    /// Within the size limit, PNG/JPEG/GIF/WebP by magic number, and decodable: your own files and imported skins.
    nonisolated static func isUsableImage(_ data: Data) -> Bool {
        data.count <= imageByteLimit && isImage(data) && NSImage(data: data) != nil
    }

    /// "png", "jpg", "gif" or "webp" by magic bytes (`isImage` has passed).
    nonisolated static func fileExtension(of data: Data) -> String {
        let b = [UInt8](data.prefix(3))
        return b == [0xFF, 0xD8, 0xFF] ? "jpg" : b == Array("GIF".utf8) ? "gif" : data.starts(with: Array("RIFF".utf8)) ? "webp" : "png"
    }

    /// PNG, JPEG, GIF or WebP by their magic bytes, the formats the web fetch accepts.
    nonisolated static func isImage(_ data: Data) -> Bool {
        let b = [UInt8](data.prefix(12))
        func starts(_ magic: [UInt8], at offset: Int = 0) -> Bool { b.count >= offset + magic.count && Array(b[offset..<offset + magic.count]) == magic }
        return starts([0x89, 0x50, 0x4E, 0x47]) || starts([0xFF, 0xD8, 0xFF]) || starts(Array("GIF8".utf8))
            || (starts(Array("RIFF".utf8)) && starts(Array("WEBP".utf8), at: 8))
    }

    private func place(_ data: Data, name: String, in skinId: String) throws {
        guard let saved = userSkins.manifest(for: skinId),
              var manifest = (try? JSONSerialization.jsonObject(with: saved)) as? [String: Any] else {
            throw UserSkinStore.StoreError.unreadableManifest(skinId)
        }
        let asset = try userSkins.addImage(data, preferredName: name, to: skinId)
        var decoration = manifest["decoration"] as? [String: Any] ?? [:]
        let imagery = decoration["imagery"] as? [Any] ?? []
        let replaced = imagery.contains { Self.placement($0) == SkinImagePlacement.header.rawValue }
        decoration["imagery"] = Self.placing(["asset": asset, "placement": SkinImagePlacement.header.rawValue, "textFree": false], in: imagery)
        manifest["decoration"] = decoration
        let updated = try JSONSerialization.data(withJSONObject: manifest, options: [.prettyPrinted, .sortedKeys])
        _ = try userSkins.save(manifest: updated, palettes: userSkins.palettes(for: skinId), for: skinId)
        reloadAndMarkScroll()
        post(replaced ? "Replaced the header image with this one." : "Added the image to the header.", to: skinId, store: true)
    }

    /// `imagery` with `entry` added, replacing any entry for the same placement: one image is drawn per placement,
    /// so the newest approval is the one shown. The replaced file stays on disk for undo (export leaves it out).
    private static func placing(_ entry: Any, in imagery: [Any]) -> [Any] {
        imagery.filter { placement($0) != placement(entry) } + [entry]
    }

    private static func placement(_ entry: Any) -> String? { (entry as? [String: Any])?["placement"] as? String }

    /// The reload rebuilds the Settings window, so mark the scroll target here, where it will be
    /// consumed, rather than on every action (a no-change action would leave it pending).
    private func reloadAndMarkScroll() {
        skinStore.scrollSettingsBack(to: SkinChatView.scrollId)
        skinStore.reloadUserSkins()
    }

    // MARK: chat lines

    /// A Whimbrel line for `skinId`'s chat: shown while that chat is open, stored when `store` and the skin is saved.
    private func post(_ text: String, to skinId: String, store: Bool) {
        let turn = SkinChatTurn(role: "whimbrel", text: text, date: Date())
        if store { try? userSkins.appendChat(turn, to: skinId) }
        if chatSkinId == skinId { turns.append(turn) }
    }
}
