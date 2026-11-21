// The server has no @types/node; this declares the slice of node:fs that the
// real-SQLite tests use (they only run in the plain-Node vitest project).
declare module "node:fs" {
  export function readFileSync(path: URL | string, encoding: "utf8"): string;
}
