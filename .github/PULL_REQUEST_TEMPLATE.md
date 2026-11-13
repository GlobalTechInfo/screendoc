## What
<!-- One or two sentences: what does this PR change, and why? -->

## Verification
<!-- What you ran. The CI gate is: cargo fmt --check, clippy -D warnings,
     full test suite, fuzz smoke. At minimum: -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` (zero warnings, no new `#[allow]`)
- [ ] `cargo test --workspace`
- [ ] Behavior covered by a test (unit or e2e) — new code ships with its test
- [ ] `./scripts/license-audit.sh` still passes (if dependencies changed)
- [ ] `docs/LICENSES.md` regenerated (if the dependency tree changed)

## Compatibility
- [ ] No breaking CLI change (or: documented below, with `--old` behavior)
- [ ] No new unpinned dependency (this repo pins exact versions; add `=x.y.z`)
- [ ] CHANGELOG.md updated for user-visible changes
