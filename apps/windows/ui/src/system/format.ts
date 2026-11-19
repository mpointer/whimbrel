// Time words for the components. No logic beyond formatting.

/** `848` -> "14:08"; an hour or more -> "1:02:03". */
export function formatClock(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(sec)}` : `${m}:${two(sec)}`;
}

/** `848` -> "14 minutes 8 seconds", `61` -> "1 minute 1 second": the spoken form, with the right plurals. */
export function spokenDuration(totalSeconds: number, locale = "en"): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const parts: [number, "hour" | "minute" | "second"][] = [
    [Math.floor(s / 3600), "hour"],
    [Math.floor((s % 3600) / 60), "minute"],
    [s % 60, "second"],
  ];
  const shown = parts.filter(([n]) => n > 0);
  if (shown.length === 0) shown.push([0, "second"]);
  return shown
    .map(([n, unit]) => new Intl.NumberFormat(locale, { style: "unit", unit, unitDisplay: "long" }).format(n))
    .join(" ");
}

/** "Sam Okafor" -> "SO"; "Jo at Northwind" -> "JN" is avoided by taking the first two words only. */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[1][0]).toUpperCase();
}
