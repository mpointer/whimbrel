import SwiftUI
import UIKit
import UserNotifications

/// This phone's name, the sync line with Sync now, and the theme choice (per device).
struct SettingsSheet: View {
    @ObservedObject var sync: SyncModel
    @ObservedObject var enrollment: EnrollmentModel
    @EnvironmentObject private var coordinator: ReminderCoordinator
    @ObservedObject private var themes = ThemeStore.shared
    @Environment(\.dismiss) private var dismiss
    @State private var approvals: DeviceApprovalModel?
    @AppStorage(AppLanguage.writeInKey) private var writeIn = ""
    @Environment(\.openURL) private var openURL
    @Environment(\.scenePhase) private var scenePhase
    /// A copy of the phone-local reminder settings; every change is written to the store and rescheduled.
    @State private var reminders: ReminderSettings?
    @State private var notificationsDenied = false

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
                remindersSection
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
            .onAppear {
                if approvals == nil { approvals = enrollment.makeApprovalModel() }
                if reminders == nil { reminders = coordinator.settings.settings }
            }
            .task { await refreshNotificationsDenied() }
            .onChange(of: scenePhase) { _, phase in
                if phase == .active { Task { await refreshNotificationsDenied() } }
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

private extension SettingsSheet {
    func refreshNotificationsDenied() async {
        let status = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
        notificationsDenied = status == .denied
    }

    /// A binding into the settings copy that saves and reschedules on every change.
    func reminderBinding<Value>(_ keyPath: WritableKeyPath<ReminderSettings, Value>) -> Binding<Value> {
        Binding(
            get: { (reminders ?? coordinator.settings.settings)[keyPath: keyPath] },
            set: { value in
                var updated = reminders ?? coordinator.settings.settings
                updated[keyPath: keyPath] = value
                reminders = updated
                coordinator.settings.settings = updated
                Task { await coordinator.reschedule() }
            })
    }

    var digestTime: Binding<Date> {
        Binding(
            get: { ReminderSettingsRows.date(forMinuteOfDay: reminderBinding(\.digestMinuteOfDay).wrappedValue) },
            set: { reminderBinding(\.digestMinuteOfDay).wrappedValue = ReminderSettingsRows.minuteOfDay(of: $0) })
    }

    @ViewBuilder var remindersSection: some View {
        let current = reminders ?? coordinator.settings.settings
        let rows = ReminderSettingsRows.make(isCloud: coordinator.isCloud, settings: current, denied: notificationsDenied)
        Section {
            if rows.showsUpgradeLines {
                Text("Reminders come with Cloud.").foregroundStyle(Color.theme(.textPrimary))
                    .listRowBackground(Color.theme(.backgroundCard))
                Text("Upgrade in Whimbrel on your computer.").foregroundStyle(Color.theme(.textSecondary))
                    .listRowBackground(Color.theme(.backgroundCard))
            }
            if rows.showsMeetingToggle {
                Toggle("Meeting reminders", isOn: reminderBinding(\.meetingEnabled))
                    .listRowBackground(Color.theme(.backgroundCard))
            }
            if rows.showsLeadPicker {
                Picker("Remind me", selection: reminderBinding(\.leadMinutes)) {
                    ForEach([5, 10, 15], id: \.self) { Text(String(localized: "\($0) minutes before")).tag($0) }
                }
                .listRowBackground(Color.theme(.backgroundCard))
            }
            if rows.showsDigestToggle {
                Toggle("Steps due today", isOn: reminderBinding(\.digestEnabled))
                    .listRowBackground(Color.theme(.backgroundCard))
            }
            if rows.showsDigestTime {
                DatePicker("At", selection: digestTime, displayedComponents: .hourAndMinute)
                    .listRowBackground(Color.theme(.backgroundCard))
            }
            if rows.showsDeniedNotice {
                Text("Notifications are off for Whimbrel. Turn them on in the Settings app.")
                    .foregroundStyle(Color.theme(.textSecondary))
                    .listRowBackground(Color.theme(.backgroundCard))
            }
        } header: {
            Text("Reminders")
        } footer: {
            if rows.showsFooter { Text("Reminders use your last sync.") }
        }
    }
}
