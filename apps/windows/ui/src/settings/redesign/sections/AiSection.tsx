import { WSettingsCard } from "../../../system";
import { t } from "../../../l10n";
import type { SectionProps } from "../types";
import { AiKeysCard, AiRoutingCard, WhimbrelAiCard } from "./ai/AiMain";
import { AiSearchModelCard } from "./ai/AiSearchModel";
import { AiTypeSafeCard } from "./ai/AiTypeSafe";
import { AiInstructionsCard, AiUsageCard } from "./ai/AiUsage";
import { useAi } from "./ai/useAi";

/** Settings > AI: Local-only mode first, then routing, Whimbrel AI, API keys, instructions, usage and caps, TypeSafe, semantic search. */
export function AiSection(_props: SectionProps) {
  const ai = useAi();
  const { settings } = ai;
  if (!settings) return <WSettingsCard title={t("AI")} headingLevel={3} loading={!ai.error} error={ai.error} />;
  return (
    <>
      <AiRoutingCard ai={ai} settings={settings} />
      <WhimbrelAiCard view={ai.whimbrel} refreshModels={ai.loadWhimbrelModels} />
      <AiKeysCard ai={ai} settings={settings} />
      <AiInstructionsCard ai={ai} settings={settings} />
      <AiUsageCard ai={ai} settings={settings} />
      <AiTypeSafeCard />
      <AiSearchModelCard />
    </>
  );
}
