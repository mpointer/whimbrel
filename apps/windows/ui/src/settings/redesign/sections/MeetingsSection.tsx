import { MeetingModelsCardRedesign } from "./meetings/MeetingModelsCardRedesign";
import { MeetingsCardRedesign } from "./meetings/MeetingsCardRedesign";
import type { SectionProps } from "../types";

/** Meetings: how calls are noticed, started and consented to, then the transcription and speaker models. */
export function MeetingsSection(_props: SectionProps) {
  return (
    <>
      <MeetingsCardRedesign />
      <MeetingModelsCardRedesign />
    </>
  );
}
