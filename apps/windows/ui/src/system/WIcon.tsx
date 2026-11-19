import type { ReactNode } from "react";

// Windows 11 Fluent line style, drawn by hand: a 20px box, a 1.5px stroke, round ends, currentColor.
// Solid parts (record dot, pause bars, stop square) use fill="currentColor" on their own element.
const FILL = { fill: "currentColor", stroke: "none" } as const;

const GLYPHS = {
  today: (
    <>
      <rect x="3" y="4.5" width="14" height="12.5" rx="2" />
      <path d="M3 8.5h14M7 3v3M13 3v3" />
      <rect x="6.5" y="11" width="3" height="3" rx=".6" {...FILL} />
    </>
  ),
  meetings: (
    <>
      <rect x="2.5" y="5.5" width="10" height="9" rx="2" />
      <path d="M12.5 9l5-2.5v7l-5-2.5" />
    </>
  ),
  library: (
    <>
      <rect x="3" y="3.5" width="3.5" height="13" rx="1" />
      <rect x="8" y="3.5" width="3.5" height="13" rx="1" />
      <path d="M13.2 5.4l2.9-.8 2.2 10.9-2.9.8z" />
    </>
  ),
  topics: <path d="M7.5 3l-1 14M13.5 3l-1 14M3.5 7.5H17M3 12.5h13.5" />,
  automations: <path d="M11 2.5L4.5 11H9l-1 6.5L15.5 9H11z" />,
  settings: (
    <>
      <path d="M3 6h7M16 6h1M3 14h1M8 14h9" />
      <circle cx="13" cy="6" r="2" />
      <circle cx="6" cy="14" r="2" />
    </>
  ),
  search: (
    <>
      <circle cx="8.5" cy="8.5" r="5.5" />
      <path d="M12.7 12.7L17 17" />
    </>
  ),
  people: (
    <>
      <circle cx="7.5" cy="7" r="3" />
      <path d="M2.5 16.5c0-3 2.2-4.8 5-4.8s5 1.8 5 4.8" />
      <circle cx="14" cy="7.8" r="2.2" />
      <path d="M14 12c2.2 0 3.7 1.4 3.7 3.8" />
    </>
  ),
  apps: (
    <>
      <rect x="3.5" y="3.5" width="5" height="5" rx="1.2" />
      <rect x="11.5" y="3.5" width="5" height="5" rx="1.2" />
      <rect x="3.5" y="11.5" width="5" height="5" rx="1.2" />
      <rect x="11.5" y="11.5" width="5" height="5" rx="1.2" />
    </>
  ),
  calendar: (
    <>
      <rect x="3" y="4.5" width="14" height="12.5" rx="2" />
      <path d="M3 8.5h14M7 3v3M13 3v3M7 12h.01M10 12h.01M13 12h.01M7 14.5h.01M10 14.5h.01" />
    </>
  ),
  clock: (
    <>
      <circle cx="10" cy="10" r="7" />
      <path d="M10 6v4.2l2.6 1.6" />
    </>
  ),
  mic: (
    <>
      <rect x="7.5" y="2.5" width="5" height="9" rx="2.5" />
      <path d="M4.5 9.5a5.5 5.5 0 0 0 11 0M10 15v2.5" />
    </>
  ),
  record: (
    <>
      <circle cx="10" cy="10" r="7" />
      <circle cx="10" cy="10" r="3.4" {...FILL} />
    </>
  ),
  pause: (
    <>
      <rect x="5.5" y="4" width="3" height="12" rx="1" {...FILL} />
      <rect x="11.5" y="4" width="3" height="12" rx="1" {...FILL} />
    </>
  ),
  play: <path d="M6.5 4l9 6-9 6z" {...FILL} />,
  stop: <rect x="5" y="5" width="10" height="10" rx="1.8" {...FILL} />,
  copy: (
    <>
      <rect x="7" y="7" width="9.5" height="9.5" rx="2" />
      <path d="M13 7V5.5a2 2 0 0 0-2-2H5.5a2 2 0 0 0-2 2V11a2 2 0 0 0 2 2H7" />
    </>
  ),
  open: <path d="M8 4.5H5.5a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V12M11.5 3.5h5v5M16.5 3.5l-7 7" />,
  check: <path d="M4 10.5l4 4 8-9" />,
  chevron: <path d="M5 7.5l5 5 5-5" />,
  chevronRight: <path d="M7.5 5l5 5-5 5" />,
  close: <path d="M5 5l10 10M15 5L5 15" />,
  spark: <path d="M10 2.5c.6 3.6 2.4 5.9 7.5 7.5-5.1 1.6-6.9 3.9-7.5 7.5-.6-3.6-2.4-5.9-7.5-7.5C7.6 8.4 9.4 6.1 10 2.5z" />,
  document: (
    <>
      <path d="M6 2.5h5.5l4 4V16a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 16V4A1.5 1.5 0 0 1 6 2.5z" />
      <path d="M11.5 2.5v4h4M7.5 10.5h5M7.5 13.5h5" />
    </>
  ),
  chat: <path d="M4 4h12a1.5 1.5 0 0 1 1.5 1.5v7A1.5 1.5 0 0 1 16 14H9l-3.5 3v-3H4a1.5 1.5 0 0 1-1.5-1.5v-7A1.5 1.5 0 0 1 4 4z" />,
  table: (
    <>
      <rect x="3" y="4" width="14" height="12" rx="1.5" />
      <path d="M3 8.5h14M3 12.5h14M8.5 8.5V16" />
    </>
  ),
  mail: (
    <>
      <rect x="2.5" y="4.5" width="15" height="11" rx="2" />
      <path d="M3 6l7 5 7-5" />
    </>
  ),
  flag: <path d="M5 17V3.5M5 4.5h9l-1.8 3 1.8 3H5" />,
  warning: <path d="M10 3l7.5 13h-15zM10 8v4M10 14.2h.01" />,
  info: (
    <>
      <circle cx="10" cy="10" r="7.5" />
      <path d="M10 9v4.5M10 6.5h.01" />
    </>
  ),
} satisfies Record<string, ReactNode>;

export type WIconName = keyof typeof GLYPHS;
export const W_ICON_NAMES = Object.keys(GLYPHS) as WIconName[];

export interface WIconProps {
  name: WIconName;
  /** Size of the box in px at 100% text; 20 is the design size. Drawn in rem, so it grows with the text size. */
  size?: number;
  /** Set only when the icon stands alone and carries meaning. Otherwise it is hidden from assistive tech. */
  label?: string;
  className?: string;
}

export function WIcon({ name, size = 20, label, className }: WIconProps) {
  return (
    <svg
      className={className ? `w-icon ${className}` : "w-icon"}
      viewBox="0 0 20 20"
      width={`${size / 16}rem`}
      height={`${size / 16}rem`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      focusable="false"
    >
      {GLYPHS[name]}
    </svg>
  );
}
