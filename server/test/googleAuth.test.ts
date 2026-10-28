import { generateKeyPair, SignJWT, exportJWK, createLocalJWKSet, type JWTVerifyGetKey } from "jose";
import { it, expect, beforeAll } from "vitest";
import { verifyGoogleIdToken, allowedAudiences } from "../src/googleAuth";

const AUDIENCE = "test-client-id";
const ISSUER = "https://accounts.google.com";

let jwks: JWTVerifyGetKey;
let privateKey: CryptoKey;

beforeAll(async () => {
  const keyPair = await generateKeyPair("RS256");
  privateKey = keyPair.privateKey;
  const publicJwk = await exportJWK(keyPair.publicKey);
  publicJwk.kid = "test-key-1";
  publicJwk.alg = "RS256";
  jwks = createLocalJWKSet({ keys: [publicJwk] });
});

async function signToken(overrides: Record<string, unknown> = {}): Promise<string> {
  const claims: Record<string, unknown> = {
    email: "user@example.com",
    email_verified: true,
    ...overrides,
  };
  delete claims.sub;
  return new SignJWT(claims)
    .setProtectedHeader({ alg: "RS256", kid: "test-key-1" })
    .setIssuer((overrides.issuer as string) ?? ISSUER)
    .setAudience((overrides.audience as string) ?? AUDIENCE)
    .setSubject((overrides.sub as string) ?? "google-sub-123")
    .setExpirationTime((overrides.expiresIn as string) ?? "1h")
    .sign(privateKey);
}

it("verifies a valid token and returns email + sub", async () => {
  const token = await signToken();
  const identity = await verifyGoogleIdToken(token, AUDIENCE, jwks);
  expect(identity).toEqual({ email: "user@example.com", googleSub: "google-sub-123", audience: AUDIENCE });
});

it("rejects a token with the wrong audience", async () => {
  const token = await signToken({ audience: "wrong-client-id" });
  await expect(verifyGoogleIdToken(token, AUDIENCE, jwks)).rejects.toThrow();
});

it("rejects a token with the wrong issuer", async () => {
  const token = await signToken({ issuer: "https://evil.example.com" });
  await expect(verifyGoogleIdToken(token, AUDIENCE, jwks)).rejects.toThrow();
});

it("rejects an expired token", async () => {
  const token = await signToken({ expiresIn: "-1h" });
  await expect(verifyGoogleIdToken(token, AUDIENCE, jwks)).rejects.toThrow();
});

it("rejects a token missing the email claim", async () => {
  const claims = { sub: "google-sub-123", email_verified: true };
  const token = await new SignJWT(claims)
    .setProtectedHeader({ alg: "RS256", kid: "test-key-1" })
    .setIssuer(ISSUER)
    .setAudience(AUDIENCE)
    .setExpirationTime("1h")
    .sign(privateKey);
  await expect(verifyGoogleIdToken(token, AUDIENCE, jwks)).rejects.toThrow();
});

it("rejects a token with an unverified email", async () => {
  const token = await signToken({ email_verified: false });
  await expect(verifyGoogleIdToken(token, AUDIENCE, jwks)).rejects.toThrow();
});

const IOS_AUDIENCE = "test-ios-client-id";

it("a token for the iOS client id is accepted when the var is set", async () => {
  const aud = allowedAudiences({ GOOGLE_CLIENT_ID: AUDIENCE, GOOGLE_IOS_CLIENT_ID: IOS_AUDIENCE });
  const token = await signToken({ audience: IOS_AUDIENCE });
  const identity = await verifyGoogleIdToken(token, aud, jwks);
  expect(identity.email).toBe("user@example.com");
});

it("a token for an unknown audience is still rejected", async () => {
  const aud = allowedAudiences({ GOOGLE_CLIENT_ID: AUDIENCE, GOOGLE_IOS_CLIENT_ID: IOS_AUDIENCE });
  const token = await signToken({ audience: "unknown-client-id" });
  await expect(verifyGoogleIdToken(token, aud, jwks)).rejects.toThrow();
});

it("an empty GOOGLE_IOS_CLIENT_ID does not widen the audience list", async () => {
  expect(allowedAudiences({ GOOGLE_CLIENT_ID: AUDIENCE, GOOGLE_IOS_CLIENT_ID: "" })).toEqual([AUDIENCE]);
  expect(allowedAudiences({ GOOGLE_CLIENT_ID: AUDIENCE })).toEqual([AUDIENCE]);
  const aud = allowedAudiences({ GOOGLE_CLIENT_ID: AUDIENCE, GOOGLE_IOS_CLIENT_ID: "" });
  const token = await signToken({ audience: IOS_AUDIENCE });
  await expect(verifyGoogleIdToken(token, aud, jwks)).rejects.toThrow();
});

it("rejects every token when the audience list is empty (fail-closed)", async () => {
  const token = await signToken();
  await expect(verifyGoogleIdToken(token, [], jwks)).rejects.toMatchObject({
    code: "ERR_JWT_CLAIM_VALIDATION_FAILED",
    claim: "aud",
  });
});

it("returns the matched audience, also when aud is an array", async () => {
  const aud = allowedAudiences({ GOOGLE_CLIENT_ID: AUDIENCE, GOOGLE_IOS_CLIENT_ID: IOS_AUDIENCE });
  expect((await verifyGoogleIdToken(await signToken({ audience: IOS_AUDIENCE }), aud, jwks)).audience).toBe(IOS_AUDIENCE);
  const multi = await new SignJWT({ email: "a@b.c", email_verified: true })
    .setProtectedHeader({ alg: "RS256", kid: "test-key-1" })
    .setIssuer(ISSUER)
    .setAudience(["other", IOS_AUDIENCE])
    .setSubject("s")
    .setExpirationTime("1h")
    .sign(privateKey);
  expect((await verifyGoogleIdToken(multi, aud, jwks)).audience).toBe(IOS_AUDIENCE);
});
