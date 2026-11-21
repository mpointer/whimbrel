import SwiftUI

/// The phone's Devices screen: new devices waiting for approval, with the comparison code to
/// check. Approving wraps this phone's group key to the new device's public key, as the Mac's
/// `SyncState.approve` does. The phone never makes a group key, so it never resets an account.
@MainActor
final class DeviceApprovalModel: ObservableObject {
    struct Row: Equatable {
        let device: SyncDeviceRecord
        let code: String?
        /// Another pending device shows the same code.
        let collides: Bool
        var statusLine: String { device.status == "resetting" ? String(localized: "Resetting…") : device.status.capitalized }
        var codeLine: String? {
            guard device.status == "pending" else { return nil }
            return code.map { String(localized: "Check this matches the code on the new device: \($0)") } ?? String(localized: "Can't verify this device")
        }
        var canApprove: Bool { device.status == "pending" && !collides }
    }

    static var collisionWarning: String { String(localized: "Another pending device shows the same code — don't approve it") }
    static let pollSeconds: UInt64 = 5

    @Published private(set) var rows: [Row] = []
    @Published private(set) var actionError: String?
    /// An approval is in flight: the button is disabled so a double tap sends one.
    @Published private(set) var approving = false
    var pendingCount: Int { rows.filter { $0.device.status == "pending" }.count }

    private let backend: EnrollmentBackend
    private let sleep: (UInt64) async throws -> Void
    private let onDevices: ([SyncDeviceRecord]) -> Void

    init(
        backend: EnrollmentBackend,
        onDevices: @escaping ([SyncDeviceRecord]) -> Void = { _ in },
        sleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0 * 1_000_000_000) }
    ) {
        self.backend = backend
        self.onDevices = onDevices
        self.sleep = sleep
    }

    /// Polls while the Devices screen is visible: the view runs this from `.task`, which SwiftUI cancels on disappear.
    func run() async {
        while !Task.isCancelled {
            await refresh()
            do { try await sleep(Self.pollSeconds) } catch { break }
        }
    }

    func refresh() async {
        guard let own = Self.ownPublicKey() else { return }
        do {
            let records = try await backend.listDevices(idToken: try await backend.idToken(), devicePublicKey: own)
            let others = records.filter { $0.publicKey != own && $0.status != "revoked" }
            let codes = others.map { CoreMemoryStore.deviceComparisonCode(publicKeyBase64: $0.publicKey) }
            let collisions = collidingPendingDeviceIDs(zip(others, codes).map { ($0.id, $0.status, $1) })
            rows = zip(others, codes).map { Row(device: $0, code: $1, collides: collisions.contains($0.id)) }
            onDevices(records)
        } catch {
            // Offline or a passing failure: keep what is shown; the next tick retries.
        }
    }

    func approve(_ device: SyncDeviceRecord) async {
        // A colliding code can't be told apart on screen, so never approve it.
        guard rows.first(where: { $0.device.id == device.id })?.canApprove ?? false else { return }
        guard !approving else { return }
        approving = true
        defer { approving = false }
        actionError = nil
        do {
            guard let own = Self.ownPublicKey(), let secret = try SyncKeychain.loadDeviceSecretKey() else { throw SyncStateError.noDeviceKeypair }
            guard let groupKey = try SyncKeychain.loadGroupKey() else { throw SyncStateError.noGroupKey }
            guard let wrapped = CoreMemoryStore.wrapSyncGroupKey(senderSecretKeyBase64: secret, recipientPublicKeyBase64: device.publicKey, groupKeyBase64: groupKey) else {
                actionError = String(localized: "Failed to wrap the group key for the new device.")
                return
            }
            try await backend.approve(idToken: try await backend.idToken(), devicePublicKey: own, targetDeviceId: device.id, wrappedGroupKey: wrapped)
            await refresh()
        } catch {
            actionError = EnrollmentModel.userMessage(for: error)
        }
    }

    private static func ownPublicKey() -> String? {
        (try? SyncKeychain.loadDeviceSecretKey()).flatMap { $0 }.flatMap { CoreMemoryStore.syncDevicePublicKey(secretKeyBase64: $0) }
    }
}

struct DeviceApprovalView: View {
    @ObservedObject var model: DeviceApprovalModel
    @ObservedObject private var themes = ThemeStore.shared

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WTokens.Space.md) {
                if let error = model.actionError { ProblemLine(text: error) }
                if model.rows.isEmpty {
                    WStateView(kind: .empty, title: String(localized: "No other devices."))
                }
                ForEach(model.rows, id: \.device.id) { row in card(row) }
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.vertical, WTokens.Space.md)
            .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .environment(\.wStrings, .app)
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Devices")
        .navigationBarTitleDisplayMode(.inline)
        .task { await model.run() }
        .id(themes.revision)
    }

    /// One device. A device waiting for approval shows the code to compare in the text color, not
    /// greyed, because it is the thing to read before tapping Approve. The button sits under the
    /// text at full width, so it keeps its size at the largest text sizes.
    private func card(_ row: DeviceApprovalModel.Row) -> some View {
        VStack(alignment: .leading, spacing: WTokens.Space.sm) {
            VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
                Text(row.device.name ?? row.device.platform ?? "Unnamed device")
                    .wType(WTokens.TypeScale.subtitle)
                    .foregroundStyle(Color.theme(.textPrimary))
                Text(row.statusLine)
                    .wType(WTokens.TypeScale.secondary)
                    .foregroundStyle(Color.theme(.textSecondary))
            }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            if let line = row.codeLine {
                Text(line)
                    .wType(WTokens.TypeScale.body)
                    .foregroundStyle(Color.theme(.textPrimary))
                    .fixedSize(horizontal: false, vertical: true)
            }
            if row.collides { ProblemLine(text: DeviceApprovalModel.collisionWarning) }
            if row.canApprove {
                Button("Approve") { Task { await model.approve(row.device) } }
                    .buttonStyle(.w(.primary, fill: true))
                    .disabled(model.approving)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(WTokens.Space.lg)
        .wCard()
    }
}

/// Something went wrong, or needs care: a warning mark and the words, never color alone.
private struct ProblemLine: View {
    let text: String

    var body: some View {
        Label(text, systemImage: "exclamationmark.triangle")
            .wType(WTokens.TypeScale.secondary)
            .foregroundStyle(Color.theme(.error))
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// At the free plan's phone limit: swap this phone in, or join the waitlist. No fresh start here.
struct DeviceLimitView: View {
    @ObservedObject var model: EnrollmentModel
    let holders: [LimitHolder]
    @State private var wants: Set<String> = []
    @State private var joined = false
    @State private var working = false

    var body: some View {
        VStack(spacing: WTokens.Space.lg) {
            Text(EnrollmentModel.limitMessage(holders: holders))
                .wType(WTokens.TypeScale.body)
                .foregroundStyle(Color.theme(.textPrimary))
            Text("Cloud lifts this limit. Subscribe from Whimbrel on your Mac or PC (Settings → Sync & Devices).")
                .wType(WTokens.TypeScale.secondary)
                .foregroundStyle(Color.theme(.textSecondary))
            if let error = model.actionError { ProblemLine(text: error) }
            if let holder = holders.first {
                Button("Use this phone instead") {
                    Task { working = true; await model.swap(replacing: holder); working = false }
                }
                .buttonStyle(.w(.primary, fill: true))
                .disabled(working)
            }
            waitlist
        }
    }

    /// The waitlist, set apart on its own card so it does not read as part of the swap above.
    private var waitlist: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.sm) {
            if joined {
                Text("You're on the waitlist. Thanks for telling us what you need.")
                    .wType(WTokens.TypeScale.body)
                    .foregroundStyle(Color.theme(.textPrimary))
            } else {
                Text("Want more? Tell us what you'd use:")
                    .wType(WTokens.TypeScale.bodyStrong)
                    .foregroundStyle(Color.theme(.textPrimary))
                    .accessibilityAddTraits(.isHeader)
                ForEach(EnrollmentModel.waitlistOptions, id: \.want) { option in
                    Toggle(isOn: Binding(
                        get: { wants.contains(option.want) },
                        set: { if $0 { wants.insert(option.want) } else { wants.remove(option.want) } }
                    )) {
                        Text(option.label).wType(WTokens.TypeScale.body).foregroundStyle(Color.theme(.textPrimary))
                    }
                    .tint(Color.theme(.accentPrimary))
                    .frame(minHeight: WTokens.Size.target)
                }
                Button("Join the waitlist") {
                    Task { working = true; joined = await model.joinWaitlist(wants: wants.sorted()); working = false }
                }
                .buttonStyle(.w(.secondary, fill: true))
                .disabled(wants.isEmpty || working)
            }
        }
        .multilineTextAlignment(.leading)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(WTokens.Space.lg)
        .wCard()
    }
}
