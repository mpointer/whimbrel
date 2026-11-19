// The Thread design system, Windows. Contract: docs/ui-redesign/design-system.md.
export { WIcon, W_ICON_NAMES, type WIconName } from "./WIcon";
export { WRoot, WButton, WIconButton, WSpeakerChip, WSearchBar, WFilterChip, WSettingsRow } from "./controls";
export { WRecordingIndicator, WLevelMeter, WCaptureState, WConsentBanner, recordingName, recordingWord, captureLabel, type WRecordingState } from "./recording";
export {
  WSidebar,
  WSidebarItem,
  WSpine,
  WSpineStop,
  WSpineNow,
  WNowMarker,
  WMeetingCard,
  WMemoryBlock,
  WEmptyState,
  WLoadingState,
  WErrorState,
} from "./layout";
export { WTranscriptView, WTranscriptLine, WNoteBlock, WSourceMark, WSourceCard, WActionItemRow } from "./notes";
export { formatClock, spokenDuration, initials } from "./format";
export {
  WBanner,
  WFlowPage,
  WFlowWait,
  WPanel,
  WTextField,
  WCheckbox,
  WNotice,
  WProgressLine,
  WNumberedList,
  WSegmented,
  WDialog,
  WMoreMenu,
  flowKeyAction,
  type FlowKeyEvent,
  type WMoreMenuItem,
} from "./flow";
export { WSectionLabel, WAskBar, WNextMeeting, type WLabelsVariant, type WAskVariant, type WNextVariant, type WAskBarProps, type WNextMeetingProps } from "./home";
