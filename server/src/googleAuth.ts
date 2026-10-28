import { createRemoteJWKSet, jwtVerify, type JWTVerifyGetKey } from "jose";

export interface GoogleIdentity {
  email: string;
  googleSub: string;
  /** The token's `aud` (first accepted entry if `aud` is an array). */
  audience: string;
}

const GOOGLE_ISSUER = "https://accounts.google.com";
const GOOGLE_JWKS_URL = "https://www.googleapis.com/oauth2/v3/certs";

let cachedJwks: JWTVerifyGetKey | undefined;

/** Memoized remote JWKS pointed at Google's real key endpoint. Only used
 * in production wiring (Task 6) — tests inject a local JWKS instead so
 * they never make a real network call. */
export function googleJwks(): JWTVerifyGetKey {
  if (!cachedJwks) {
    cachedJwks = createRemoteJWKSet(new URL(GOOGLE_JWKS_URL));
  }
  return cachedJwks;
}

/** Accepted token audiences: the desktop client and, if configured, the iOS
 * client. Empty/unset values are dropped so a blank var can never widen
 * acceptance (reject-by-default). */
export function allowedAudiences(env: {
  GOOGLE_CLIENT_ID: string;
  GOOGLE_IOS_CLIENT_ID?: string;
}): string[] {
  return [env.GOOGLE_CLIENT_ID, env.GOOGLE_IOS_CLIENT_ID].filter(
    (a): a is string => typeof a === "string" && a !== "",
  );
}

/** Verifies a Google-issued ID token's signature, issuer, audience, and
 * expiry, and extracts the identity claims this Worker cares about. The
 * native app completes the OAuth flow directly with Google (PKCE) and
 * hands this Worker only the resulting ID token — no code exchange
 * happens here. Restricted to RS256 (Google's actual signing algorithm)
 * as defense in depth, rather than accepting whatever algorithm the JWKS
 * resolver happens to find a matching key for. Requires `email_verified`
 * — the sign-in allowlist gates on the `email` claim, so an unverified
 * email must never be trusted as an identity, even though this isn't
 * currently exploitable while the allowlist is gmail.com-only. */
export async function verifyGoogleIdToken(
  idToken: string,
  expectedAudience: string | string[],
  jwks: JWTVerifyGetKey,
): Promise<GoogleIdentity> {
  const { payload } = await jwtVerify(idToken, jwks, {
    issuer: GOOGLE_ISSUER,
    audience: expectedAudience,
    algorithms: ["RS256"],
  });
  if (typeof payload.email !== "string" || typeof payload.sub !== "string") {
    throw new Error("Google ID token missing required email/sub claims");
  }
  if (payload.email_verified !== true) {
    throw new Error("Google ID token email is not verified");
  }
  const accepted = [expectedAudience].flat();
  const audience = [payload.aud].flat().find((a) => a !== undefined && accepted.includes(a));
  if (audience === undefined) throw new Error("Google ID token has no accepted audience");
  return { email: payload.email, googleSub: payload.sub, audience };
}
