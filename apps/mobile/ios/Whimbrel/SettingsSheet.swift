import SwiftUI
import UIKit

/// This phone's name, the sync line with Sync now, and the theme choice (per device).
struct SettingsSheet: View {
    @ObservedObject var sync: SyncModel
    @ObservedObject private var themes = ThemeStore.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("This iPhone") {
                    Text(UIDevice.current.name).foregroundStyle(Color.theme(.textPrimary))
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
