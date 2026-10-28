import SwiftUI

/// FR-ONB-1: checklist UI with deep links to System Settings, one permission at a time.
struct OnboardingView: View {
    @EnvironmentObject var permissions: PermissionsManager

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Grant Whimbrel access")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)

            Text("Whimbrel runs with partial access — each permission below unlocks one capability. Grant them one at a time; you can revisit this anytime from the menu bar.")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            VStack(alignment: .leading, spacing: Spacing.md) {
                ForEach(PermissionKind.allCases) { kind in
                    PermissionRow(
                        kind: kind,
                        granted: permissions.granted(for: kind),
                        onGrant: { permissions.request(kind) },
                        onOpenSettings: { permissions.openSystemSettings(for: kind) }
                    )
                }
            }

            Spacer()

            HStack {
                Text("\(grantedCount) of \(PermissionKind.allCases.count) granted")
                    .font(.whimbrelBody(.footnote))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Button("Refresh") { permissions.refresh() }
                    .font(.whimbrelBody(.footnote))
                    .buttonStyle(.link)
                Spacer()
                if permissions.allGranted {
                    Label("All set", systemImage: "checkmark.seal.fill")
                        .font(.whimbrelBody(.footnote).bold())
                        .foregroundStyle(Color.whimbrelSuccess)
                }
            }
        }
        .padding(Spacing.xl)
        .onAppear { permissions.refresh() }
    }

    private var grantedCount: Int {
        PermissionKind.allCases.filter { permissions.granted(for: $0) }.count
    }
}

private struct PermissionRow: View {
    let kind: PermissionKind
    let granted: Bool
    let onGrant: () -> Void
    let onOpenSettings: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            Image(systemName: granted ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(granted ? .green : Color.whimbrelTextSecondary)
                .font(.title3)

            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text(kind.title).font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
                Text(kind.explanation)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                if !granted {
                    HStack(spacing: Spacing.sm) {
                        Button("Grant Access", action: onGrant)
                        Button("Open System Settings", action: onOpenSettings)
                    }
                    .font(.whimbrelBody(.caption))
                    .buttonStyle(.link)
                }
            }
        }
    }
}
