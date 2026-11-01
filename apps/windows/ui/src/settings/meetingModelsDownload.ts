// The meeting models' download, shared by the launch download and Settings → Meeting models
// (docs/superpowers/plans/2026-11-01-windows-setup-in-one-screen.md).

import { meetingModelsDownloadStart, meetingModelsStatus } from "../api";
import type { MeetingModelsStatus } from "../generated/types";
import { downloadStore } from "./downloadStore";

export const meetingModelsDownload = downloadStore<MeetingModelsStatus>(meetingModelsStatus, meetingModelsDownloadStart);
