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

/// FR-ONB-1 for a new Mac: the two permissions a first meeting needs, once, and nothing else.
struct FirstRunView: View {
    @EnvironmentObject var permissions: PermissionsManager
    let done: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Welcome to Whimbrel").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Two permissions and you're ready for your first meeting. No bot joins the call, and your recordings stay on this Mac.")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(PermissionKind.firstRun) { kind in
                PermissionRow(
                    kind: kind,
                    granted: permissions.granted(for: kind),
                    onGrant: { permissions.request(kind) },
                    onOpenSettings: { permissions.openSystemSettings(for: kind) }
                )
            }
            Text("Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("Anything else, Whimbrel asks for when you first use it. Its models are downloading in the background.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Get Started", action: done).buttonStyle(.whimbrelPrimary)
            }
        }
        .padding(Spacing.xl)
        .frame(width: 480)
    }
}

/// A permission asked for from inside the feature that needs it.
struct PermissionCard: View {
    @EnvironmentObject var permissions: PermissionsManager
    let kind: PermissionKind

    var body: some View {
        if !permissions.granted(for: kind) {
            PermissionRow(
                kind: kind,
                granted: false,
                onGrant: { permissions.request(kind) },
                onOpenSettings: { permissions.openSystemSettings(for: kind) }
            )
            .padding(Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
        }
    }
}

struct PermissionRow: View {
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
