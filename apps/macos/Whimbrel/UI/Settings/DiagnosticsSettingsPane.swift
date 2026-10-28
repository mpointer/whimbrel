import SwiftUI

struct DiagnosticsSettingsPane: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Diagnostics").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Developer tools. Only Debug builds show this pane.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            Button("Print Diagnostics to Console") { DiagnosticsPrinter.printSigningAndPermissionState() }
                .buttonStyle(.whimbrelSecondary)
            Button("Run Core Self-Test") {
                let store = appState.coreStore
                CoreMemoryStore.accessQueue.async {
                    CoreSelfTest.run(store: store, deviceId: appState.deviceId)
                }
            }
                .buttonStyle(.whimbrelSecondary)
        }
    }
}
