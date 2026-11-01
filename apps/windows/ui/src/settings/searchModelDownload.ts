// The search model's download, shared by the launch download and Settings → Semantic search
// (docs/superpowers/plans/2026-11-01-windows-setup-in-one-screen.md).

import { modelDownloadStart, modelStatus } from "../api";
import type { ModelStatus } from "../generated/types";
import { downloadStore } from "./downloadStore";

export const searchModelDownload = downloadStore<ModelStatus>(modelStatus, modelDownloadStart);
