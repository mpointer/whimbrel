// WCAG contrast, ported from core/skins/src/contrast.rs so the browser matrix judges text the way the shell does.

export type Rgb = [number, number, number];

const linear = (channel: number): number => {
  const c = channel / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
};

export const relativeLuminance = ([r, g, b]: Rgb): number => 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);

/** WCAG contrast ratio, 1 to 21. */
export function ratio(a: Rgb, b: Rgb): number {
  const [la, lb] = [relativeLuminance(a), relativeLuminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** `fg` at `alpha` over opaque `bg`, per sRGB channel. */
export const composite = (fg: Rgb, alpha: number, bg: Rgb): Rgb => [0, 1, 2].map((i) => Math.round(fg[i] * alpha + bg[i] * (1 - alpha))) as Rgb;

/** A computed CSS colour as the browser reports it: `rgb()`, `rgba()` or `color(srgb r g b / a)`. */
export function parseColor(text: string): { rgb: Rgb; alpha: number } | null {
  const legacy = /^rgba?\(\s*([\d.]+)[ ,]+([\d.]+)[ ,]+([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/.exec(text.trim());
  if (legacy) return { rgb: [Number(legacy[1]), Number(legacy[2]), Number(legacy[3])], alpha: alphaOf(legacy[4]) };
  const srgb = /^color\(srgb\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)(?:\s*\/\s*([\d.]+%?))?\s*\)$/.exec(text.trim());
  if (srgb) return { rgb: [Number(srgb[1]) * 255, Number(srgb[2]) * 255, Number(srgb[3]) * 255].map(Math.round) as Rgb, alpha: alphaOf(srgb[4]) };
  return null;
}

const alphaOf = (text?: string): number => (text === undefined ? 1 : text.endsWith("%") ? Number.parseFloat(text) / 100 : Number(text));
