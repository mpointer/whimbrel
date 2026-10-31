import SwiftUI
import UIKit

/// This phone's name, the sync line with Sync now, and the theme choice (per device).
struct SettingsSheet: View {
    @ObservedObject var sync: SyncModel
    @ObservedObject var enrollment: EnrollmentModel
    @ObservedObject private var themes = ThemeStore.shared
    @Environment(\.dismiss) private var dismiss
    @State private var approvals: DeviceApprovalModel?
    @AppStorage(AppLanguage.writeInKey) private var writeIn = ""
    @Environment(\.openURL) private var openURL

    var body: some View {
        NavigationStack {
            List {
                Section("This iPhone") {
                    Text(UIDevice.current.name).foregroundStyle(Color.theme(.textPrimary))
                        .listRowBackground(Color.theme(.backgroundCard))
                }
                if let approvals {
                    Section("Devices") {
                        NavigationLink {
                            DeviceApprovalView(model: approvals)
                        } label: {
                            HStack {
                                Text("Devices").foregroundStyle(Color.theme(.textPrimary))
                                Spacer()
                                if enrollment.pendingDeviceCount > 0 {
                                    Text("\(enrollment.pendingDeviceCount)")
                                        .font(.caption.bold())
                                        .foregroundStyle(.white)
                                        .padding(.horizontal, 8).padding(.vertical, 2)
                                        .background(Capsule().fill(Color.theme(.accentPrimary)))
                                        .accessibilityLabel("\(enrollment.pendingDeviceCount) waiting for approval")
                                }
                            }
                        }
                        .listRowBackground(Color.theme(.backgroundCard))
                    }
                }
                Section("Language") {
                    Button("Language") {
                        if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                    }
                    .listRowBackground(Color.theme(.backgroundCard))
                    Picker("Write notes and summaries in", selection: $writeIn) {
                        Text("Same as app language").tag("")
                        ForEach(AppLanguage.writeInOptions, id: \.self) {
                            Text(verbatim: Locale.current.localizedString(forIdentifier: $0) ?? $0).tag($0)
                        }
                    }
                    .listRowBackground(Color.theme(.backgroundCard))
                }
                Section("Sync") {
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        Text(ReadLogic.syncLine(lastSyncedAt: sync.lastSyncedAt, now: context.date, isSyncing: sync.isSyncing, error: sync.lastError))
                            .foregroundStyle(Color.theme(.textSecondary))
                    }
                    .listRowBackground(Color.theme(.backgroundCard))
                    Button("Sync now") { Task { await sync.syncNow(manual: true) } }
                        .disabled(sync.isSyncing)
                        .listRowBackground(Color.theme(.backgroundCard))
                }
                Section("Theme") {
                    ForEach(themes.themes) { theme in
                        Button {
                            themes.select(theme.id)
                        } label: {
                            HStack {
                                Text(theme.name).foregroundStyle(Color.theme(.textPrimary))
                                Spacer()
                                if theme.id == themes.selectedId {
                                    Image(systemName: "checkmark").foregroundStyle(Color.theme(.accentPrimary))
                                }
                            }
                        }
                        .listRowBackground(Color.theme(.backgroundCard))
                    }
                }
            }
            .onAppear { if approvals == nil { approvals = enrollment.makeApprovalModel() } }
            .scrollContentBackground(.hidden)
            .background(Color.theme(.backgroundPage).ignoresSafeArea())
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .tint(Color.theme(.accentPrimary))
        .id(themes.revision)
    }
}
