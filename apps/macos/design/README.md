# apps/macos/design

Vector source for Whimbrel's app icon and menu-bar glyph. The icon is the whimbrel in
profile, cream on a rust tile, with the long down-curved bill as the signature. Below
64 px the full bird gets too thin, so the small sizes use a crop of the head, neck and a
heavier bill. The compiled assets Xcode actually uses live in `Whimbrel/Assets.xcassets/`;
these SVGs are the editable source if the mark needs to change.

| File | Use |
|---|---|
| `app-icon.svg` | Mac app icon, 64 px and up (macOS icon grid: tile inset, with shadow) |
| `app-icon-small.svg` | Mac app icon, 16 and 32 px |
| `app-icon-flat.svg` | Same art with no grid margin: Windows installer, taskbar and window icons |
| `app-icon-flat-small.svg` | Small crop with no grid margin: Windows `.ico` sizes up to 32 px |
| `menubar-glyph.svg` | Menu-bar status glyph (template image) |

The website's marks (`website/demo/whimbrel-mark*.svg`) use the same art.

Regenerate the PNGs (`rsvg-convert`, `brew install librsvg`):

```bash
cd apps/macos/design
A=../Whimbrel/Assets.xcassets/AppIcon.appiconset
rsvg-convert -w 16 -h 16 app-icon-small.svg -o $A/icon_16x16.png
rsvg-convert -w 32 -h 32 app-icon-small.svg -o $A/icon_16x16@2x.png
rsvg-convert -w 32 -h 32 app-icon-small.svg -o $A/icon_32x32.png
for pair in 32x32@2x:64 128x128:128 128x128@2x:256 256x256:256 256x256@2x:512 512x512:512 512x512@2x:1024; do
  rsvg-convert -w ${pair#*:} -h ${pair#*:} app-icon.svg -o "$A/icon_${pair%:*}.png"
done
for sz in 18 36; do
  rsvg-convert -w $sz -h $sz menubar-glyph.svg -o "menubar-${sz}.png"
done
```

Copy the menu-bar renders to `Whimbrel/Assets.xcassets/StatusGlyph.imageset/`
(`status-glyph.png` = 18px @1x, `status-glyph@2x.png` = 36px @2x — keep
`template-rendering-intent: template` in that imageset's `Contents.json` so macOS
auto-tints it for the light/dark menu bar, same as an SF Symbol).
