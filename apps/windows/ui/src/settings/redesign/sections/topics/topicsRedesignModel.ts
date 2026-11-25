// Checks and wording for the redesigned Topics section. The old topicsModel.ts stays the source of parsing and labels.
import type { TopicSeedView } from "../../../../generated/types";
import { t } from "../../../../l10n";

/** A sentence when `name` cannot be a topic: the same name (ignoring case) as another topic. `ignoreKey` is the topic being edited. */
export function topicNameProblem(name: string, seeds: readonly Pick<TopicSeedView, "key" | "name">[], ignoreKey: string | null): string | null {
  const wanted = name.trim().toLowerCase();
  if (!wanted) return null;
  const clash = seeds.find((s) => s.key !== ignoreKey && s.name.trim().toLowerCase() === wanted);
  return clash ? t("{name} is already a topic.", { name: clash.name }) : null;
}

/** A sentence when `name` is already one of the names the person goes by (ignoring case), else null. */
export function ownNameProblem(name: string, names: readonly string[]): string | null {
  const wanted = name.trim().toLowerCase();
  if (!wanted) return null;
  const clash = names.find((n) => n.trim().toLowerCase() === wanted);
  return clash ? t("{value} is already in the list.", { value: clash }) : null;
}
