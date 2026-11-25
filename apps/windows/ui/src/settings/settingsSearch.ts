import type { SettingsSectionId } from "./settingsSections";

/**
 * One searchable setting (the Mac's SettingsSearchEntry). `id` is also the row's
 * `data-settings-anchor`. Titles repeat the control's English label; keywords are the other words
 * people reach for. Setting values are never indexed. Search text is English only, as on the Mac.
 */
export interface SettingsSearchEntry {
  id: string;
  title: string;
  section: Exclude<SettingsSectionId, "status">;
  keywords: string[];
}

const e = (id: string, title: string, section: SettingsSearchEntry["section"], keywords: string[] = []): SettingsSearchEntry => ({
  id,
  title,
  section,
  keywords,
});

/** Only what exists on Windows: no Automations or Diagnostics cards. */
export const SETTINGS_SEARCH_ENTRIES: readonly SettingsSearchEntry[] = [
  // General
  e("general.general", "General", "general", ["language", "locale", "translate", "write notes in", "start at sign-in", "startup", "check for updates", "update", "version"]),
  e("general.appearance", "Appearance", "general", ["dark mode", "light mode", "theme", "skin", "gallery", "colors", "make a skin", "look"]),
  e("general.about", "About this PC", "general", ["version", "build", "data folder", "logs"]),
  // Profile
  e("profile.profile", "Profile", "profile", ["about me", "personalize", "context", "name", "role", "learned facts", "suggestions"]),
  // Plan & billing
  e("plan.summary", "Plan & billing", "plan", ["plus", "cloud", "subscription", "refund", "invoice", "cancel", "upgrade", "price", "payment"]),
  e("plan.manage", "Manage subscription", "plan", ["cancel", "refund", "invoice", "payment method", "stripe", "billing portal"]),
  e("plan.provider", "Use Whimbrel AI as your provider", "plan", ["allowance", "credit", "plus ai", "hosted ai"]),
  // Capture & Privacy
  e("capture.capture", "Activity capture", "capture", ["screen text", "ocr", "privacy", "pause", "capture", "show screen captures"]),
  e("capture.localOnly", "Local-only mode", "capture", ["offline", "private", "no cloud", "privacy"]),
  e("capture.hotkey", "Hotkey panel", "capture", ["shortcut", "keyboard", "global", "ask"]),
  e("capture.retention", "Keeping data", "capture", ["retention", "delete old", "keep", "history", "days", "purge", "expire"]),
  e("capture.storage", "Storage", "capture", ["free up space", "disk", "space", "compress", "recordings", "audio", "media"]),
  e("capture.encryption", "Database encryption", "capture", ["encrypt", "passphrase", "database", "protect", "password", "recovery key"]),
  e("capture.foreignFiles", "Files kept aside", "capture", ["foreign files", "unknown files", "data folder"]),
  e("capture.obsidian", "Obsidian", "capture", ["vault", "notes", "markdown", "export"]),
  e("capture.delete", "Delete everything", "capture", ["erase", "wipe", "remove data", "reset"]),
  // Meetings
  e("meetings.meetings", "Meetings", "meetings", ["recording", "consent", "notice", "auto-start", "calendar", "live transcript", "snapshots", "slides", "voices", "speaker"]),
  e("meetings.models", "Meeting models", "meetings", ["transcription", "whisper", "model", "quality", "speed", "accuracy", "diarization", "download"]),
  // AI
  e("ai.localOnly", "Local-only mode", "ai", ["offline", "private", "ollama", "no cloud", "privacy"]),
  e("ai.provider", "Preferred provider", "ai", ["default provider", "model", "routing", "which ai", "llm"]),
  e("ai.whimbrelAI", "Whimbrel AI", "ai", ["allowance", "credit", "balance", "plus", "prepaid", "top up"]),
  e("ai.keys", "API keys", "ai", ["api key", "anthropic", "claude", "openai", "gpt", "google", "gemini", "openrouter", "key", "budget"]),
  e("ai.instructions", "Instructions", "ai", ["prompts", "custom", "tune", "reset"]),
  e("ai.usage", "Model usage", "ai", ["cost", "tokens", "spending", "spend today", "daily cap", "budget"]),
  e("ai.typesafe", "TypeSafe", "ai", ["reply priorities", "email priority", "rank", "reply"]),
  e("ai.semanticSearch", "Semantic search", "ai", ["embeddings", "search model", "index"]),
  // Topics
  e("topics.topics", "Topics", "topics", ["customers", "projects", "products", "clients", "add topic", "aliases"]),
  e("topics.names", "Names I go by", "topics", ["my name", "nicknames", "steps"]),
  // Sync & Devices
  e("devices.phone", "Whimbrel on your phone", "devices", ["iphone", "android", "mobile", "phone", "beta", "qr"]),
  e("devices.sync", "Sync", "devices", ["sign in", "google", "account", "approve", "turn on sync"]),
  e("devices.list", "Devices on this account", "devices", ["devices", "rename", "device name", "revoke", "approve"]),
  // Connectors
  e("connectors.accounts", "Connected accounts", "connectors", ["google calendar", "calendar", "accounts", "connect", "add account"]),
  e("connectors.gmail", "Gmail", "connectors", ["email", "mail", "google"]),
  e("connectors.ownClient", "Use your own Google Cloud project", "connectors", ["oauth", "client id", "client secret"]),
  e("connectors.mcp", "AI assistants (MCP)", "connectors", ["claude desktop", "token", "server", "assistant", "revoke"]),
];

/** Lower case with accents removed, so "Résumé" finds "resume" and the other way round. */
export function fold(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

/**
 * Entries matching `query` in the title, a keyword or the section's title: case- and accent-
 * insensitive, plain substring (the query is never a regular expression). Title matches come first.
 * An empty or blank query matches nothing (the normal section list shows then).
 */
export function searchSettings(query: string, sectionTitle: (id: SettingsSearchEntry["section"]) => string): SettingsSearchEntry[] {
  const q = fold(query.trim());
  if (!q) return [];
  const titleHits: SettingsSearchEntry[] = [];
  const otherHits: SettingsSearchEntry[] = [];
  for (const entry of SETTINGS_SEARCH_ENTRIES) {
    if (fold(entry.title).includes(q)) titleHits.push(entry);
    else if (entry.keywords.some((k) => fold(k).includes(q)) || fold(sectionTitle(entry.section)).includes(q)) otherHits.push(entry);
  }
  return [...titleHits, ...otherHits];
}
