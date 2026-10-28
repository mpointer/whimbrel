import Foundation

/// The allowed values for every skin axis, sent to the skin maker with each turn so the model
/// picks from the same lists `SkinLoader` and `ThemeLoader` accept. Built from the Swift
/// constants, so it cannot drift from the loaders.
enum SkinCatalogue {
    static func object() -> [String: Any] {
        func raw<E: CaseIterable & RawRepresentable>(_: E.Type) -> [String] where E.RawValue == String { E.allCases.map(\.rawValue) }
        let tokens = ThemeToken.allCases.map(\.rawValue)
        return [
            "manifest": [
                "shape": #"{"id","name","palettes":[palette ids],"typography":{"display","body","numerals","label","caps"},"shape":{"radius","border","borderWidth","density"},"icons":{role: SF Symbol name},"decoration":{"headerBand","dividers","texture","imagery":[{"asset","placement","textFree"}]},"voice":{key: text},"today":{"archetype","options":{}},"motion"}"#,
                "imagery": "only images already in the current manifest; new images are added by the user approving them",
            ],
            "fonts": [
                "families": raw(SkinFamily.self),
                "weights": SkinFace.weights.keys.sorted(),
                "format": "\"Family\", \"Family/weight\" or \"Family/weight/italic\"",
            ],
            "typography": ["caps": raw(SkinCaps.self)],
            "shape": [
                "radius": ["min": Double(SkinShape.radiusRange.lowerBound), "max": Double(SkinShape.radiusRange.upperBound)],
                "borderWidth": ["min": Double(SkinShape.widthRange.lowerBound), "max": Double(SkinShape.widthRange.upperBound)],
                "border": raw(SkinBorder.self),
                "density": raw(SkinDensity.self),
            ],
            "icons": ["roles": raw(SkinIconRole.self), "values": "SF Symbol names"],
            "decoration": [
                "headerBand": raw(SkinHeaderBand.self),
                "dividers": raw(SkinDivider.self),
                "texture": raw(SkinTexture.self),
                "imageryPlacement": raw(SkinImagePlacement.self),
            ],
            "voice": [
                "keys": Dictionary(uniqueKeysWithValues: SkinVoiceKey.allCases.map { ($0.rawValue, ["limit": $0.limit]) }),
                "name": "greetings may use the literal {name}; the limit counts it as \(SkinVoiceKey.measuredNameLength) characters",
            ],
            "archetypes": raw(SkinArchetype.self),
            "motion": raw(SkinMotion.self),
            "palette": [
                "shape": ##"{"id","name","light":{token:"#rrggbb"},"dark":{token:"#rrggbb"}}; light, dark or both"##,
                "tokens": tokens,
                "colour": "#rrggbb only",
                "contrast": [
                    "minimum": ThemeContrast.minimum,
                    "pairs": ThemeContrast.requiredPairs.map { "\($0.0.rawValue) on \($0.1.rawValue)" },
                ],
            ],
        ]
    }

    static func json() -> Data {
        (try? JSONSerialization.data(withJSONObject: object(), options: [.sortedKeys])) ?? Data("{}".utf8)
    }
}
