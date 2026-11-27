// A skin card's picture of Home (richer skins row 11, docs/superpowers/plans/2026-11-25-richer-skins.md section 8). It draws
// the real Home's own parts (the next meeting, the time line, section labels, dividers, the header band and the skin's
// images) from fixed sample text, in the skin's own palette, fonts, shape and element choices. It is a still picture: no
// timers, no data from the shell, no motion, no buttons, and it is hidden from the accessibility tree, so the card keeps
// its name and tagline as its label.

import { Fragment, memo, type CSSProperties } from "react";
import { useL10n } from "../../l10n";
import { paletteColors, paletteMode, paletteToDraw, skinAttributes, skinTexture, skinVariables, textureBackground, TEXTURE_VARIABLE, type SkinDto, type SkinPalette } from "../../skin";
import { waitingLabel } from "../../automations/automationsModel";
import { timeLabel } from "../../library/rows";
import { WButton, WMeetingCard, WNextMeeting, WSectionLabel, WSpine, WSpineNow, WSpineStop } from "../../system";
import type { Appearance } from "../../theme";
import { SkinImage } from "../skinImage";
import { greetingFor, imageryOf, localizedVoice, voiceText } from "../layoutModel";
import { elementsFromOptions } from "./elements";
import { withDividers } from "./withDividers";
import "./skinPreview.css";

/** The sample day: a fixed morning, so a preview never shows the person's data or the clock. */
const SAMPLE_NOW = new Date(2026, 9, 20, 10, 30);
const at = (hour: number, minute = 0): number => new Date(2026, 9, 20, hour, minute).getTime();

/** The names and meetings a card shows. Fixed text, the same for every skin. */
export const SAMPLE = {
  name: "Alex",
  next: { title: "Design review", startUnixMs: at(11), attendees: ["Priya Nair", "Tom Becker"] },
  earlier: { title: "Pipeline sync", attendees: ["Dana Cole", "Lee Ortiz"], startUnixMs: at(9) },
  threads: [
    { id: 1, subject: "Q3 pricing", from: "Sam" },
    { id: 2, subject: "Vendor contract", from: "Jo" },
  ],
  approvals: 2,
  recap: "Pricing is settled for the pilot. Two follow-ups are with you.",
};

/** The custom properties a card sets on its picture: the skin's variables with motion off, its palette's colours for the
 *  mode it shows, and the texture, which the contrast rules in skinPreview.css switch off for forced colors and more contrast. */
export function previewStyle(dto: SkinDto, palette: SkinPalette, mode: "light" | "dark"): CSSProperties {
  const vars: Record<string, string> = { ...skinVariables(dto.skin, true), ...paletteColors(palette, mode) };
  const page = (palette[mode] ?? palette.light ?? palette.dark)?.backgroundPage;
  const texture = skinTexture(dto.skin);
  if (texture !== "none" && page) vars[TEXTURE_VARIABLE] = textureBackground(texture, page);
  return vars as CSSProperties;
}

export interface SkinPreviewHomeProps {
  dto: SkinDto;
  /** The palette the card shows: the skin's first, or the one in force for the selected skin. */
  palette: SkinPalette;
  appearance: Appearance;
  systemDark: boolean;
  /** Windows asks for more contrast: the card shows Thread's High Contrast palette, as the page does. */
  contrastMore?: boolean;
}

/** Memoised: a card redraws only when its skin, palette or appearance changes, never because another card did. */
export const SkinPreviewHome = memo(function SkinPreviewHome({ dto, palette: picked, appearance, systemDark, contrastMore = false }: SkinPreviewHomeProps) {
  const { t, locale, lang } = useL10n();
  const skin = dto.skin;
  const palette = paletteToDraw(picked, contrastMore);
  const mode = paletteMode(palette, appearance, systemDark);
  const el = elementsFromOptions(skin.today.options, skin.today.archetype);
  const voice = localizedVoice(skin, lang);
  const imagery = imageryOf(skin, dto.imageScrims);
  const links = el.actions === "links";
  const label = (words: string) => <WSectionLabel variant={el.labels}>{words}</WSectionLabel>;

  const parts = [
    <section key="line" className="th-timeline">
      {label(t("Today"))}
      <WSpine label={t("Today, in time order")} variant={el.timeline}>
        <WSpineStop time={timeLabel(SAMPLE.next.startUnixMs, locale)} node="upcoming" hero>
          <WNextMeeting
            variant={el.nextMeeting}
            links={links}
            still
            title={SAMPLE.next.title}
            time={timeLabel(SAMPLE.next.startUnixMs, locale)}
            soon={t("in {minutes} min", { minutes: 30 })}
            when={t("Today, {time}", { time: timeLabel(SAMPLE.next.startUnixMs, locale) })}
            attendees={SAMPLE.next.attendees}
            primaryLabel={t("Start")}
            secondaryLabel={t("Open brief")}
          />
        </WSpineStop>
        <WSpineNow time={timeLabel(SAMPLE_NOW.getTime(), locale)} />
        <WSpineStop time={timeLabel(SAMPLE.earlier.startUnixMs, locale)} node="past" connector="solid" last>
          <WMeetingCard variant="past" links={links} still title={SAMPLE.earlier.title} attendees={SAMPLE.earlier.attendees} primaryLabel={t("Open")} />
        </WSpineStop>
      </WSpine>
    </section>,
    <section key="approvals" className="th-section">
      {label(t("Waiting for approval"))}
      <div className="th-row">
        <span>{waitingLabel(SAMPLE.approvals)}</span>
        <WButton link={links} still>{t("Open")}</WButton>
      </div>
    </section>,
    <section key="threads" className="th-section">
      {label(t("Open threads"))}
      <ul className="th-list">
        {SAMPLE.threads.map((th) => (
          <li key={th.id}>
            <span className="th-list-title">{th.subject}</span>
            <span className="th-list-meta">{t("Waiting on you · last from {name}", { name: th.from })}</span>
          </li>
        ))}
      </ul>
    </section>,
    <section key="recap" className="th-section">
      {label(voiceText(voice, "recapHeader"))}
      <p>{SAMPLE.recap}</p>
    </section>,
  ];

  return (
    <div className="skin-preview-frame" aria-hidden="true">
      <div className="skin-preview w-sys" style={previewStyle(dto, palette, mode)} {...skinAttributes(skin)} data-theme={mode} data-palette={palette.id}>
          <div className="skin-preview-band" />
          <SkinImage imagery={imagery} skinId={skin.id} placement="header" />
          <div className="today th-home" data-next={el.nextMeeting} data-timeline={el.timeline} data-labels={el.labels} data-actions={el.actions} data-dividers={skin.decoration.dividers !== "none" ? skin.decoration.dividers : undefined}>
            <SkinImage imagery={imagery} skinId={skin.id} placement="badge" />
            <p className="th-title">{greetingFor(SAMPLE.name, SAMPLE_NOW, voice, locale).title}</p>
            {withDividers(parts, skin.decoration.dividers !== "none").map((part, i) => (
              <Fragment key={i}>{part}</Fragment>
            ))}
          </div>
      </div>
    </div>
  );
});
