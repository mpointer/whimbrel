# Shared test vectors

`tests/vectors.json` is a verbatim copy of `test/vectors.json` from the
sealed-fields repository (`/Volumes/Scratch/repos/sealed-fields`, package
version 0.6.1, branch `main`).

Source commit: a8f45c5f3a9c2723170e6d4d89a9e5e7bcc2f051
SHA-256: 14787640aa714bd84f937c00366ae4d66d39cf8a1eeeb9bed6edcb339cc7b932

The `SHA-256:` line above is read by the test
`vectors_file_matches_recorded_source_hash`. To refresh the vectors, copy the
new file, update the commit and hash here in the same change, and re-run
`cargo test -p sealed --lib --tests`.

The hash was taken from the working-tree file; the sealed-fields working tree
was not checked for uncommitted changes to that file (git was not run inside
that repo).

## Cross-language check (2026-11-14, by hand)

Rust `sealed` port against sealed-fields 0.6.1 built from source
(`src/` compiled with `tsc` into a scratch directory; the checked-in `dist/`
is stale and predates context binding, so it was not used). Key is the vector
key (`bytes(range(32))`), version `v1`, context `["users", "email"]`.

- Rust seal, Node unseal: `hello — 你好` round-tripped. PASS.
- Node seal, Rust unseal: `node → rust ✓` round-tripped. PASS.

The checked-in vectors (python_reference_padded, typescript_impl, python_impl,
and the two contextTokens) also unseal in Rust via
`shared_vectors_unseal_to_expected_plaintext`.

Note: the checked-in `dist/` of sealed-fields is out of date relative to
`src/` (0.6.x). Anyone installing it from the local path gets a version
without context binding.
