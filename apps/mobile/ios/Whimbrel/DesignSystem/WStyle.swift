import SwiftUI
import UIKit

/// Hand-written glue over the generated tokens (Tokens.generated.swift): how a fixed color and a
/// type role become SwiftUI values. Skinnable colors stay with `Color.theme(_:)`.
extension WFixedColor {
    /// The variant for an appearance. `highContrast` is the system's Increase Contrast setting.
    func value(dark: Bool, highContrast: Bool) -> UInt32 {
        switch (dark, highContrast) {
        case (false, false): light
        case (true, false): self.dark
        case (false, true): highContrastLight
        case (true, true): highContrastDark
        }
    }

    /// Follows light and dark, and the system's Increase Contrast setting, at render time.
    var color: Color {
        Color(uiColor: UIColor { traits in
            let rgb = value(dark: traits.userInterfaceStyle == .dark, highContrast: traits.accessibilityContrast == .high)
            return UIColor(red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
                           blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
        })
    }
}

/// Applies a type role so it scales with Dynamic Type, up to the largest accessibility sizes.
private struct WTypeModifier: ViewModifier {
    @ScaledMetric private var size: CGFloat
    @ScaledMetric private var lineHeight: CGFloat
    private let role: WTokens.TypeRole
    private let design: Font.Design

    init(role: WTokens.TypeRole, design: Font.Design) {
        self.role = role
        self.design = design
        _size = ScaledMetric(wrappedValue: role.size, relativeTo: role.relativeTo)
        _lineHeight = ScaledMetric(wrappedValue: role.lineHeight, relativeTo: role.relativeTo)
    }

    func body(content: Content) -> some View {
        content
            .font(.system(size: size, weight: role.weight, design: design))
            .lineSpacing(max(0, lineHeight - size * 1.2))
    }
}

extension View {
    /// A type role in the UI face (SF Pro).
    func wType(_ role: WTokens.TypeRole) -> some View { modifier(WTypeModifier(role: role, design: .default)) }
    /// A type role in the reading face (New York), for notes and answers.
    func wReading(_ role: WTokens.TypeRole) -> some View { modifier(WTypeModifier(role: role, design: .serif)) }
    /// At least the minimum tap target in both directions.
    func wTapTarget() -> some View { frame(minWidth: WTokens.Size.target, minHeight: WTokens.Size.target) }
}
