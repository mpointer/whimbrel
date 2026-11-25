// Settings → General, drawn on the design system's parts. The order is the Mac redesign's: General, Language,
// Appearance, Skin, Make a skin, then About this PC.
import { useState } from "react";
import type { SectionProps } from "../types";
import { AboutThisPcCard, GeneralSettingsCard, LanguageCard } from "./general/GeneralCards";
import { AppearanceCard, MakeSkinCard, SkinGalleryCard } from "./general/SkinCards";

export function GeneralSection({ onSection }: SectionProps) {
  // A chat turn writes back to the skin it started for, so the skin actions wait while one runs.
  const [turnRunning, setTurnRunning] = useState(false);
  return (
    <>
      <GeneralSettingsCard />
      <LanguageCard />
      <AppearanceCard />
      <SkinGalleryCard turnRunning={turnRunning} />
      <MakeSkinCard onRunning={setTurnRunning} onSetUp={() => onSection("ai")} />
      <AboutThisPcCard />
    </>
  );
}
