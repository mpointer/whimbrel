import SwiftUI

struct GeneralSettingsPane: View {
    @EnvironmentObject var appState: AppState
    var openAISettings: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("General").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)

            LabeledContent("App version") {
                Text(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown")
            }
            LabeledContent("Core version") {
                Text(CoreMemoryStore.coreVersion)
            }

            Divider().overlay(Color.whimbrelBorder)
            AppearanceThemeSettings(appearance: appState.appearanceSettings, skinChat: appState.skinChat, openAISettings: openAISettings)
        }
    }
}
