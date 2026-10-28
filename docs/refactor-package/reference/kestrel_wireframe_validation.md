# Kestrel Wireframe Validation Notes

Two visual checks were performed on the deterministic SVG wireframes. The composition shows the intended sidebar, central canvas, compact header, bounded cards, and callout structure. The sandbox SVG preview does not render text glyphs reliably, including after changing the declared font to installed DejaVu Sans. The SVG source itself contains complete accessible English labels and is intended as source-controlled wireframe output; it should be opened in a standard macOS browser, Figma, Sketch, Illustrator, or Xcode asset workflow for final typography review.

The wireframes have been designed to avoid coordinate-dependent content overlap: the accompanying UX specification defines their SwiftUI adaptive layout rules, width classes, minimum/ideal/maximum region widths, and overflow behavior. The visual artifacts are structural layouts, not final production artwork.
