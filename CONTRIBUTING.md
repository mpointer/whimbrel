# Contributing

Thanks for looking. This is an early-access product with one maintainer, so the
rules are few and they are about keeping the tree honest.

## Before you start

- Open an issue first for anything bigger than a bug fix, so the design is agreed
  before the code exists.
- Check the README's platform table and `docs/05-roadmap.md` for what is built and
  what is planned. Pull requests that make a document claim more than the code
  does will be sent back.
- The public repository is published from a private one at each release (see the
  README). A pull request here is reviewed here, applied there, and lands with the
  next release, so expect it to arrive as part of a release commit rather than as
  its own merge.

## Building and testing

- Rust core: `cd core && cargo build && cargo test --workspace --exclude sync-engine`.
  The `mcp-host` and `ffi` tests need `npm ci` in `core/mcp-host/tests/fixtures`
  first (see `core/README.md`). `sync-engine` is build-only on Linux and Windows
  for the linker reason explained in `.github/workflows/rust-tests.yml`.
- macOS app: `apps/macos/README.md`. Windows app: `apps/windows/README.md`.
- Workers: `npx --yes npm@11 ci && npm test && npm run typecheck` in `server/` and
  `server-updates/`.
- Dependency licenses: `cd core && cargo deny check licenses sources`.

CI runs the hosted jobs on every pull request. The self-hosted macOS and Windows
jobs run only for branches in this repository, never for pull requests from forks,
so a green fork PR still gets one more run on those platforms after merge.

## Pull requests

- One topic per branch and pull request. Say in the description what broke, why,
  and how the change addresses it; that narrative is what the reviewer reads.
- Keep user-visible behaviour changes out of refactors.
- Documentation changes that describe shipped behaviour must point at the code.

## Developer Certificate of Origin

Contributions are accepted under the Apache License 2.0 (section 5: your
contribution is licensed under the same terms, no separate agreement). Sign off
each commit to certify the [Developer Certificate of Origin](https://developercertificate.org/):

```
git commit -s
```

## Security

Do not open public issues for vulnerabilities. See `SECURITY.md`.
