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
        var statusLine: String { device.status == "resetting" ? "Resetting…" : device.status.capitalized }
        var codeLine: String? {
            guard device.status == "pending" else { return nil }
            return code.map { "Check this matches the code on the new device: \($0)" } ?? "Can't verify this device"
        }
        var canApprove: Bool { device.status == "pending" && !collides }
    }

    static let collisionWarning = "Another pending device shows the same code — don't approve it"
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
                actionError = "Failed to wrap the group key for the new device."
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
        List {
            if let error = model.actionError {
                Text(error).foregroundStyle(Color.theme(.textSecondary))
                    .listRowBackground(Color.theme(.backgroundCard))
            }
            if model.rows.isEmpty {
                Text("No other devices.").foregroundStyle(Color.theme(.textSecondary))
                    .listRowBackground(Color.theme(.backgroundCard))
            }
            ForEach(model.rows, id: \.device.id) { row in
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(row.device.name ?? row.device.platform ?? "Unnamed device").foregroundStyle(Color.theme(.textPrimary))
                        Text(row.statusLine).font(.caption).foregroundStyle(Color.theme(.textSecondary))
                        if let line = row.codeLine {
                            Text(line).font(.caption).foregroundStyle(Color.theme(.textSecondary))
                        }
                        if row.collides {
                            Text(DeviceApprovalModel.collisionWarning).font(.caption).foregroundStyle(Color.theme(.textPrimary))
                        }
                    }
                    Spacer()
                    if row.canApprove {
                        Button("Approve") { Task { await model.approve(row.device) } }
                            .buttonStyle(.bordered)
                            .disabled(model.approving)
                    }
                }
                .listRowBackground(Color.theme(.backgroundCard))
            }
        }
        .scrollContentBackground(.hidden)
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Devices")
        .navigationBarTitleDisplayMode(.inline)
        .task { await model.run() }
        .id(themes.revision)
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
        VStack(spacing: 16) {
            Text(EnrollmentModel.limitMessage(holders: holders)).foregroundStyle(Color.theme(.textPrimary))
            if let error = model.actionError {
                Text(error).font(.footnote).foregroundStyle(Color.theme(.textSecondary))
            }
            if let holder = holders.first {
                Button("Use this phone instead") {
                    Task { working = true; await model.swap(replacing: holder); working = false }
                }
                .buttonStyle(.borderedProminent)
                .tint(Color.theme(.accentPrimary))
                .disabled(working)
            }
            Divider()
            if joined {
                Text("You're on the waitlist. Thanks for telling us what you need.").foregroundStyle(Color.theme(.textSecondary))
            } else {
                Text("Want more? Tell us what you'd use:").foregroundStyle(Color.theme(.textSecondary))
                ForEach(EnrollmentModel.waitlistOptions, id: \.want) { option in
                    Toggle(option.label, isOn: Binding(
                        get: { wants.contains(option.want) },
                        set: { if $0 { wants.insert(option.want) } else { wants.remove(option.want) } }
                    ))
                    .foregroundStyle(Color.theme(.textPrimary))
                }
                Button("Join the waitlist") {
                    Task { working = true; joined = await model.joinWaitlist(wants: wants.sorted()); working = false }
                }
                .buttonStyle(.bordered)
                .disabled(wants.isEmpty || working)
            }
        }
    }
}
