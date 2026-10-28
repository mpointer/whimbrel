import SwiftUI

/// Wraps a just-generated client token so it can drive a `.sheet(item:)` — the token
/// itself isn't `Identifiable`, and this is the only place in the app that needs to show
/// it, so a plain wrapper here is simpler than adding conformance to a shared model.
struct GeneratedMcpToken: Identifiable {
    let id: Int64
    let token: String
}

/// Task 6 (FR-MCPS-4's Settings-facing half): add/name/scope MCP client tokens and
/// revoke them, following `DevicesSettingsPane`'s exact structural pattern (header +
/// caption, `ForEach`-based list with row actions, a confirmation `.alert` for the
/// destructive revoke). Unlike devices, there's no live polling here — the token list
/// only changes in response to this pane's own add/revoke actions, so a plain
/// `.onAppear` refresh (matching `DevicesSettingsPane`'s own initial-load convention) is
/// enough.
struct MCPClientsSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var clients: [CoreMcpClientTokenInfo] = []
    @State private var clientPendingRevocation: CoreMcpClientTokenInfo?
    @State private var newClientName = ""
    @State private var selectedScopes: Set<String> = []
    @State private var generatedToken: GeneratedMcpToken?
    @State private var errorMessage: String?

    /// "write" is reserved for a future pass per the design spec — the underlying scope
    /// model supports the string, but it isn't offered here yet.
    private static let availableScopes = ["memory", "meetings", "transcripts"]

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("MCP Clients")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Let a local MCP client — an AI assistant, for example — read your Whimbrel data through a scoped, revocable token. Each token is shown once, right after you create it, so save it somewhere safe.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            Divider().overlay(Color.whimbrelBorder)

            serverToggleSection

            Divider().overlay(Color.whimbrelBorder)

            addClientForm

            Divider().overlay(Color.whimbrelBorder)

            clientList
        }
        .onAppear { refreshClients() }
        .alert("Revoke this client's access?", isPresented: Binding(
            get: { clientPendingRevocation != nil },
            set: { if !$0 { clientPendingRevocation = nil } }
        )) {
            Button("Revoke", role: .destructive) {
                if let client = clientPendingRevocation {
                    revoke(client)
                }
                clientPendingRevocation = nil
            }
            Button("Cancel", role: .cancel) {
                clientPendingRevocation = nil
            }
        } message: {
            Text("This client will lose access immediately. This can't be undone from here — it would need a brand new token to reconnect.")
        }
        .sheet(item: $generatedToken) { generated in
            generatedTokenSheet(generated)
        }
    }

    /// Task 7: starts/stops the real `whimbrel-mcp-server` binary as a persistent child
    /// process (`McpServerProcessManager`, owned by `AppState`). Clients still need a
    /// token from `addClientForm`/`clientList` below to actually authenticate against it
    /// once it's running.
    @ViewBuilder
    private var serverToggleSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Toggle("Enable MCP Server", isOn: Binding(
                get: { appState.mcpServerProcess.enabled },
                set: { appState.mcpServerProcess.enabled = $0 }
            ))
            .toggleStyle(.switch)

            Text(
                appState.mcpServerProcess.isRunning
                    ? "Running at http://127.0.0.1:4319/mcp"
                    : "Not running."
            )
            .font(.whimbrelBody(.caption))
            .foregroundStyle(Color.whimbrelTextSecondary)

            if let lastError = appState.mcpServerProcess.lastError {
                Text(lastError)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }
        }
    }

    @ViewBuilder
    private var addClientForm: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Add Client").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)

            TextField("Client name (e.g. \"Claude Desktop\")", text: $newClientName)
                .textFieldStyle(.roundedBorder)

            HStack(spacing: Spacing.lg) {
                ForEach(Self.availableScopes, id: \.self) { scope in
                    Toggle(scope.capitalized, isOn: Binding(
                        get: { selectedScopes.contains(scope) },
                        set: { isOn in
                            if isOn { selectedScopes.insert(scope) } else { selectedScopes.remove(scope) }
                        }
                    ))
                    .toggleStyle(.checkbox)
                }
            }

            if let errorMessage {
                Text(errorMessage)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            Button("Add Client") { addClient() }
                .buttonStyle(.whimbrelSecondary)
                .disabled(newClientName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || selectedScopes.isEmpty)
        }
    }

    @ViewBuilder
    private var clientList: some View {
        if clients.isEmpty {
            Text("No MCP clients yet.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            ForEach(clients) { client in
                HStack {
                    VStack(alignment: .leading) {
                        Text(client.name)
                            .font(.whimbrelBody(.body))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        Text("\(client.scopes.joined(separator: ", ")) · created \(formattedDate(client.createdAtUnixMs))")
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                    Spacer()
                    if let revokedAtUnixMs = client.revokedAtUnixMs {
                        Text("Revoked \(formattedDate(revokedAtUnixMs))")
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    } else {
                        Button("Revoke") {
                            clientPendingRevocation = client
                        }
                        .buttonStyle(.whimbrelSecondary)
                        .tint(Color.whimbrelError)
                    }
                }
                Divider().overlay(Color.whimbrelBorder)
            }
        }
    }

    @ViewBuilder
    private func generatedTokenSheet(_ generated: GeneratedMcpToken) -> some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Client token created")
                .font(.whimbrelDisplay(.title3))
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("This is the only time this token will be shown. Copy it now — Whimbrel only ever stores its hash, so it can't be recovered later.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            Text(generated.token)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .padding(Spacing.md)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.whimbrelBackgroundCard)
                .clipShape(RoundedRectangle(cornerRadius: Radius.control))
            HStack {
                Button("Copy to Clipboard") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(generated.token, forType: .string)
                }
                .buttonStyle(.whimbrelSecondary)
                Spacer()
                Button("Done") { generatedToken = nil }
                    .buttonStyle(.whimbrelPrimary)
            }
        }
        .padding(Spacing.xl)
        .frame(minWidth: 420)
    }

    private func formattedDate(_ unixMs: Int64) -> String {
        Date(timeIntervalSince1970: Double(unixMs) / 1000).formatted(date: .abbreviated, time: .omitted)
    }

    private func refreshClients() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let result = store.listMcpClientTokens()
            DispatchQueue.main.async {
                clients = result ?? []
            }
        }
    }

    private func addClient() {
        guard let store = appState.coreStore else {
            errorMessage = "Add failed: store unavailable."
            return
        }
        let name = newClientName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !selectedScopes.isEmpty else { return }
        let scopes = Array(selectedScopes)
        CoreMemoryStore.accessQueue.async {
            let result = store.generateMcpClientToken(name: name, scopes: scopes)
            DispatchQueue.main.async {
                guard let result else {
                    errorMessage = "Failed to create client token."
                    return
                }
                errorMessage = nil
                newClientName = ""
                selectedScopes = []
                generatedToken = GeneratedMcpToken(id: result.id, token: result.token)
                refreshClients()
            }
        }
    }

    private func revoke(_ client: CoreMcpClientTokenInfo) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            store.revokeMcpClientToken(id: client.id)
            DispatchQueue.main.async {
                refreshClients()
            }
        }
    }
}
