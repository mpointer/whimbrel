import SwiftUI

/// Appearance (System / Light / Dark), the skin gallery and the skin chat, in General settings. Per device.
struct AppearanceThemeSettings: View {
    @ObservedObject var appearance: AppearanceSettings
    let skinChat: SkinChatSession
    /// Opens the AI settings (the chat's "Set up an AI provider").
    let openAISettings: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Appearance").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            Picker("Appearance", selection: $appearance.choice) {
                ForEach(AppearanceChoice.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 320)
            .accessibilityIdentifier("appearance-picker")
            .disabled(appearance.lockedCaption != nil)
            if let caption = appearance.lockedCaption {
                Text(caption).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }

            Text("Skin").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                .padding(.top, Spacing.sm)
            SkinGallery(skinChat: skinChat)

            Text("Make a skin").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                .padding(.top, Spacing.sm)
            SkinChatView(session: skinChat, setUp: openAISettings)
        }
    }
}
