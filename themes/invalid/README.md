# Theme loader test corpus

Shared by the Mac (`ThemeLoaderTests`) and Windows (`themes.test.ts`) loaders. Each file and its expected outcome:

- `not-json.json`: malformed JSON, skipped.
- `no-id.json`: no `id`, skipped.
- `no-variants.json`: has `id` and `name` but neither `light` nor `dark`, skipped.
- `bad-hex.json`: a color that is not `#rrggbb`, the whole file is skipped.
- `low-contrast.json`: loads; its light variant fails the 4.5:1 guard and is replaced by Default's light variant, its dark variant is kept.
- `missing-tokens.json`: loads; only `accentPrimary` is overridden, every other token falls back to Default, and the single (light) variant serves both appearances.
- `single-variant.json`: loads; only a dark variant, which serves light mode too, missing tokens come from Default's dark variant.
- `legacy-12-token.json`: loads; a theme written before `selection`, `highlight` and `textOnAccent` existed uses Default's values for those three.

See docs/superpowers/specs/2026-10-06-themes-design.md.
