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

/** True when the Worker var OPEN_SIGN_IN is exactly "true" (trimmed,
 * case-insensitive). Only the hosted relay sets it. */
export function isOpenSignIn(env: { OPEN_SIGN_IN?: string }): boolean {
  return env.OPEN_SIGN_IN?.trim().toLowerCase() === "true";
}

/** Sign-in is open to any verified Google account only when OPEN_SIGN_IN is
 * on; otherwise the allowlist gates as before. Flipping the var back
 * reverses the rollout. */
export function isSignInAllowed(
  email: string,
  env: { OPEN_SIGN_IN?: string; ALLOWED_GOOGLE_EMAILS?: string },
): boolean {
  return isOpenSignIn(env) || isEmailAllowed(email, env.ALLOWED_GOOGLE_EMAILS);
}

/** Accounts in the UNLIMITED_EMAILS CSV skip the free device limit. Same
 * matching rules (and fail-closed default) as the allowlist. */
export function isUnlimited(email: string, csv: string | undefined): boolean {
  return isEmailAllowed(email, csv);
}

/** The free device limit applies only on the hosted relay (OPEN_SIGN_IN on);
 * a self-hosted relay has none. UNLIMITED_EMAILS are exempt either way. */
export function hasDeviceLimit(email: string, env: { OPEN_SIGN_IN?: string; UNLIMITED_EMAILS?: string }): boolean {
  return isOpenSignIn(env) && !isUnlimited(email, env.UNLIMITED_EMAILS);
}
