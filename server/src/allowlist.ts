/** Checks a verified Google account email against a comma-separated
 * allowlist. Fails closed: an undefined, empty, or whitespace-only
 * allowlist denies every email — requiring explicit configuration is the
 * correct default for something gating who can create an account at all.
 * Comparison is case-insensitive and tolerates whitespace around entries. */
export function isEmailAllowed(email: string, allowedEmailsCsv: string | undefined): boolean {
  if (!allowedEmailsCsv || allowedEmailsCsv.trim().length === 0) {
    return false;
  }
  const allowed = allowedEmailsCsv
    .split(",")
    .map((entry) => entry.trim().toLowerCase())
    .filter((entry) => entry.length > 0);
  return allowed.includes(email.trim().toLowerCase());
}
