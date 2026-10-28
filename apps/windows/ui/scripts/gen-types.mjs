// Turns src/generated/schema.json (written by `cargo run --bin gen-schema` in
// src-tauri) into src/generated/types.ts. `--check` fails instead of writing when
// types.ts is stale — the UI half of plan Decision 11's "a shape change breaks the
// build". (The Rust half is src-tauri's `committed_schema_is_current` test.)

import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { compile } from "json-schema-to-typescript";

const schemaPath = fileURLToPath(new URL("../src/generated/schema.json", import.meta.url));
const typesPath = fileURLToPath(new URL("../src/generated/types.ts", import.meta.url));

const schema = JSON.parse(await readFile(schemaPath, "utf8"));
const body = await compile(schema, "WhimbrelDtos", {
  bannerComment: "",
  unreachableDefinitions: true,
  additionalProperties: false,
  format: true,
  style: { printWidth: 100 },
});
const header =
  "// Generated from apps/windows/src-tauri's command DTOs by `npm run gen:types`.\n" +
  "// Do not edit. Change the Rust type, run `cargo run --bin gen-schema`, then `npm run gen:types`.\n\n";
const text = header + body;

if (process.argv.includes("--check")) {
  const current = await readFile(typesPath, "utf8").catch(() => "");
  if (current.replace(/\r\n/g, "\n") !== text) {
    console.error("src/generated/types.ts is stale: run `npm run gen:types`.");
    process.exit(1);
  }
  console.log("types.ts is current.");
} else {
  await writeFile(typesPath, text);
  console.log(`wrote ${typesPath}`);
}
