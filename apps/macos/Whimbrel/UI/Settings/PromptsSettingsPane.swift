import SwiftUI

struct PromptsSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var selectedSurface: PromptSurface = .ask

    var body: some View {
        HStack(alignment: .top, spacing: Spacing.xl) {
            List(PromptSurface.allCases, selection: $selectedSurface) { surface in
                Text(surface.displayName).tag(surface)
            }
            .frame(width: 180)
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.whimbrelBorder, lineWidth: 1))

            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text("Prompts").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
                Text("Tune the instructions folded into each surface's prompt. Each surface is independent and syncs across your devices.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)

                Divider().overlay(Color.whimbrelBorder)

                Text(selectedSurface.displayName).font(.whimbrelBody(.callout))
                TextEditor(text: Binding(
                    get: { appState.promptLibrary.instructions[selectedSurface] ?? "" },
                    set: { appState.promptLibrary.instructions[selectedSurface] = $0 }
                ))
                .font(.whimbrelBody(.body))
                .frame(minHeight: 120, maxHeight: 220)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.whimbrelBorder, lineWidth: 1))
                Text(selectedSurface.helpText)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                Button("Reset to Default") {
                    appState.promptLibrary.instructions[selectedSurface] = ""
                }
                .buttonStyle(.whimbrelSecondary)
                .disabled((appState.promptLibrary.instructions[selectedSurface] ?? "").isEmpty)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
