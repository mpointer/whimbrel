import { t } from "../l10n";

export function App() {
  // l10n-ignore
  const badge = <span>Beta</span>;
  return (
    <div>
      <h1>{t("Today")}</h1>
      <p>Bare text here</p>
      {badge}
    </div>
  );
}
