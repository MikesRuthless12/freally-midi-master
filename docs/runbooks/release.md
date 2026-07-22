# Release runbook

How a release is cut, and what to do when one goes wrong.

## Before tagging

Everything in PRD § 7.6 must already be green on `main`:

- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`
- `npm run typecheck && npm run lint && npm run test && npm run build`
- `npm run test:e2e`
- `npm run dataset:validate`
- CI green on all three OS runners

Update `CHANGELOG.md` **before** tagging. The release job extracts the tagged
version's section from it for the updater's release notes, so a missing entry
means every user sees a generic note instead of what actually changed.

## Cutting a release

```bash
# Version numbers must agree, or the updater compares the wrong thing.
#   package.json .version
#   src-tauri/tauri.conf.json .version
#   Cargo.toml [workspace.package] version

git tag v0.1.0
git push origin v0.1.0
```

Pushing the tag triggers `release.yml`, which builds on all three OSes, uploads
installers as a **draft** release, then generates `SHA256SUMS`.

Publishing the draft triggers `verify-downloads.yml`.

## The rule that matters

**A release is not done until `verify-downloads` is green.** Do not announce,
do not link it anywhere, do not post the clip. A broken installer is worse than
no release: it burns the one chance a new user gives the product.

## When verification fails

1. **Fix the cause.** Read which asset failed and why — a checksum mismatch, an
   archive that will not open, a missing platform, a malformed `latest.json`.
2. **Bump the version and re-tag.** Not the same tag. The updater compares the
   manifest's version against the installed one, so re-releasing under an
   existing version reaches **nobody** — every existing user is told they are
   already current.
3. **Re-release**, and let `verify-downloads` run again.
4. Repeat until green.

Delete the failed release and its tag once the replacement is verified, so
nobody downloads the broken one from the releases list.

## Signing keys

The updater will not accept an unsigned manifest, so the release job needs two
repository secrets:

| Secret | Source |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | `~/.tauri/freally-midi-master.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | `~/.tauri/freally-midi-master.password.txt` |

**Never rotate this key.** The public half is compiled into every shipped
build; replacing it orphans every existing install, because their updater will
reject manifests signed by the new key. If it is ever compromised, that is a
new-installer-required event, not a routine rotation.

Installers are unsigned by policy — code-signing certificates cost money and
this project spends nothing. The release notes therefore carry the per-OS
first-run steps (SmartScreen, right-click→Open, `chmod +x`).

## Common failures

| Symptom | Cause |
| --- | --- |
| `latest.json is missing` | `includeUpdaterJson` was off, or no platform built |
| A platform's asset is absent | That OS's build leg failed — check the matrix |
| Asset under 1 MB | A build produced a stub; the checksum matches the wrong bytes |
| Updater sees nothing | Re-tagged an existing version instead of bumping |
| Generic release notes | `CHANGELOG.md` had no section for the tagged version |
