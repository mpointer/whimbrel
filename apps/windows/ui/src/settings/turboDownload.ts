// large-v3-turbo's download (W-1.2), shared by the offer in Library and Settings → Meeting models.
// Its status is the Quality picker's, which says whether turbo is on disk.

import { transcriptionQualityStatus, turboDownloadStart } from "../api";
import type { TranscriptionQualityStatus } from "../generated/types";
import { downloadStore } from "./downloadStore";

export const turboDownload = downloadStore<TranscriptionQualityStatus>(transcriptionQualityStatus, turboDownloadStart);
