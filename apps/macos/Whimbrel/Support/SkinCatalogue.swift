import Foundation
import WhimbrelUI

/// The allowed values for every skin axis, sent to the skin maker with each turn so the model
/// picks from the same lists `SkinLoader` and `ThemeLoader` accept. Built from the Swift
/// constants, so it cannot drift from the loaders.
enum SkinCatalogue {
    /// With `redesign` on, the five Home element choices are added under `today`; off, the output is what it has always been.
    static func object(redesign: Bool = false) -> [String: Any] {
        func raw<E: CaseIterable & RawRepresentable>(_: E.Type) -> [String] where E.RawValue == String { E.allCases.map(\.rawValue) }
        let tokens = ThemeToken.allCases.map(\.rawValue)
        var catalogue: [String: Any] = [
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
                // The background placement is dropped in Thread: it loads from old files but is not offered.
                "imageryPlacement": [SkinImagePlacement.header, .badge].map(\.rawValue),
            ],
            "voice": [
                "keys": Dictionary(uniqueKeysWithValues: SkinVoiceKey.allCases.map { ($0.rawValue, ["limit": $0.limit]) }),
                "name": "greetings may use the literal {name}; the limit counts it as \(SkinVoiceKey.measuredNameLength) characters",
            ],
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
        if redesign {
            catalogue["today"] = [
                "options": Dictionary(uniqueKeysWithValues: WHomeElements.keys.map { ($0.key, $0.values) }),
                "rule": "set today.options to any of these keys; a key left out keeps the old archetype's choice, an unknown value is ignored",
            ]
        }
        return catalogue
    }

    static func json(redesign: Bool = false) -> Data {
        (try? JSONSerialization.data(withJSONObject: object(redesign: redesign), options: [.sortedKeys])) ?? Data("{}".utf8)
    }
}
