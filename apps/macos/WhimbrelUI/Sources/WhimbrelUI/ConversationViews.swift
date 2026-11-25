import SwiftUI

// The header of the app's conversation page (docs/ui-redesign/screens/conversations.md): the title row, then the scope line and the
// note under it. Presentation only. The app passes the title row and both lines, already in the user's language, so the
// gallery can draw the same header with made-up data.

/// A conversation page's header: `top` (the title row), then `scopeLine` ("Asking about Apps: Excel, Dates: Nov 9 to Nov 15",
/// in the filter chips' own words) and `note` (for example that the People filter was not kept). A nil line is not drawn.
public struct WConversationHeader<Top: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let scopeLine: String?
    let note: String?
    let top: Top

    public init(scopeLine: String?, note: String?, @ViewBuilder top: () -> Top) {
        self.scopeLine = scopeLine
        self.note = note
        self.top = top()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(4)) {
            top
            WConversationScopeLines(scopeLine: scopeLine, note: note)
        }
        .frame(maxWidth: 760 * k, alignment: .leading)
        .padding(.horizontal, theme.space(28))
        .padding(.vertical, theme.space(14))
        .frame(maxWidth: .infinity, alignment: .center)
    }
}

/// The scope line and the note under it, as two rows of the enclosing vertical stack (a `Group`, so no extra spacing when
/// both are nil).
public struct WConversationScopeLines: View {
    @Environment(\.wTheme) private var theme
    let scopeLine: String?
    let note: String?

    public init(scopeLine: String?, note: String?) {
        self.scopeLine = scopeLine
        self.note = note
    }

    public var body: some View {
        Group {
            if let scopeLine {
                Text(verbatim: scopeLine).wFont(.secondary).foregroundStyle(theme.textSecondary)
            }
            if let note {
                Text(verbatim: note).wFont(.caption).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
