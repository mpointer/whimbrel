import type { ButtonHTMLAttributes, ReactNode } from "react";
import { t } from "../l10n";
import { initials } from "./format";
import { WIcon, type WIconName } from "./WIcon";
import "./system.css";

/** Hosts the system's text, resets and focus ring. Put it once around a screen. */
export function WRoot({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={className ? `w-sys ${className}` : "w-sys"}>{children}</div>;
}

export interface WButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className"> {
  variant?: "primary" | "secondary" | "quiet" | "destructive";
  icon?: WIconName;
  /** A shortcut hint such as "Ctrl+Enter". Shown on the button and read as part of its name. */
  hint?: string;
  /** Drawn as a text link (the skin's `actions: links` choice). Same button, same name, at least 24 by 24. */
  link?: boolean;
  /** Part of a still picture of a screen (a skin card's preview): drawn as the same box with no button in it, since a
   *  button cannot sit inside the card's own button and a picture has nothing to press. */
  still?: boolean;
}

export function WButton({ variant = "secondary", icon, hint, link, still, children, type = "button", ...rest }: WButtonProps) {
  if (still) {
    return (
      <span className={link ? `w-button w-button-${variant} w-button-link` : `w-button w-button-${variant}`}>
        {icon ? <WIcon name={icon} /> : null}
        <span>{children}</span>
        {hint ? <span className="w-button-hint">{hint}</span> : null}
      </span>
    );
  }
  return (
    <button type={type} className={link ? `w-button w-button-${variant} w-button-link` : `w-button w-button-${variant}`} {...rest}>
      {icon ? <WIcon name={icon} /> : null}
      <span>{children}</span>
      {hint ? <span className="w-button-hint">{hint}</span> : null}
    </button>
  );
}

export function WIconButton({ icon, label, ...rest }: { icon: WIconName; label: string } & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button type="button" className="w-icon-button" aria-label={label} {...rest}>
      <WIcon name={icon} />
    </button>
  );
}

export function WSpeakerChip({ name, size = "regular" }: { name: string; size?: "small" | "regular" }) {
  return (
    <span className={size === "small" ? "w-speaker w-speaker-small" : "w-speaker"}>
      <span className="w-speaker-avatar" aria-hidden="true">
        {initials(name)}
      </span>
      <span>{name}</span>
    </span>
  );
}

export interface WSearchBarProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  /** Shortcut hint shown at the end while the field is empty. */
  hint?: string;
}

export function WSearchBar({ value, onChange, placeholder, hint = "Ctrl+K" }: WSearchBarProps) {
  return (
    <div className="w-search" role="search">
      <WIcon name="search" />
      <input
        type="search"
        value={value}
        aria-label={t("Search your work memory")}
        placeholder={placeholder ?? t("Search your work memory")}
        onChange={(e) => onChange(e.target.value)}
      />
      {value ? (
        <WIconButton icon="close" label={t("Clear search")} onClick={() => onChange("")} />
      ) : (
        <span className="w-kbd">{hint}</span>
      )}
    </div>
  );
}

export interface WFilterChipProps {
  label: string;
  icon?: WIconName;
  /** A filter that opens a menu shows a chevron. */
  menu?: boolean;
  selected?: boolean;
  onPress: () => void;
  /** When set, the chip can be removed, and a Remove button appears beside it. */
  onRemove?: () => void;
  /** For a chip that opens a menu: whether it is open. */
  expanded?: boolean;
}

export function WFilterChip({ label, icon, menu, selected, onPress, onRemove, expanded }: WFilterChipProps) {
  return (
    <span className={selected ? "w-chip w-chip-selected" : "w-chip"}>
      <button
        type="button"
        className="w-chip-main"
        aria-pressed={menu ? undefined : !!selected}
        aria-haspopup={menu ? "menu" : undefined}
        aria-expanded={expanded}
        onClick={onPress}
      >
        {icon ? <WIcon name={icon} size={16} /> : null}
        <span>{label}</span>
        {menu ? <WIcon name="chevron" size={14} /> : null}
      </button>
      {onRemove ? (
        <button type="button" className="w-chip-remove" aria-label={t("Remove filter {name}", { name: label })} onClick={onRemove}>
          <WIcon name="close" size={14} />
        </button>
      ) : null}
    </span>
  );
}
