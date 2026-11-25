// Phrases the redesigned Memory & Privacy section must not show (tests use this). The redesigned cards say "your
// library" in their own t() keys; nothing here rewrites text, because an English-only rewrite leaves other languages
// with the old words.

export const BANNED_PHRASES = [/memory database/i, /Activity capture/i, /Capture & Privacy/i, /\bChats\b/, /Never delete/i, /Keep full quality forever/i];

export function bannedIn(text: string): string[] {
  return BANNED_PHRASES.filter((re) => re.test(text)).map((re) => re.source);
}
