# Security

We take the security of screendoc seriously. That matters more than usual
for this project: **sessions record terminal output, which may contain
secrets** — and the tool optionally encrypts that data at rest.

## Reporting a vulnerability

- **Do not open a public issue.** Use the private vulnerability reporting
  form: [Report a vulnerability](https://github.com/GlobalTechInfo/screendoc/security/advisories/new).
- Include: the affected version, a minimal reproduction (prefer synthetic
  data — never attach real credentials), and, if you have one, a sketch of
  the impact.
- If you prefer email, ask maintainers for their PGP key first and
  encrypt any samples.

## Response policy

- Acknowledgement: within 7 days.
- Triage: within 14 days (accepted / not-a-vulnerability / duplicate).
- Fix target: within 30 days of acceptance (sooner for key-exposure and
  remote-execution classes).
- When a fix ships: a GitHub Security Advisory + changelog entry, with
  credit to the reporter unless they ask otherwise.

## Threat model notes (for reporters and contributors)

- **Stored data:** `~/.screendoc/sessions/<id>/` — content files are
  owner-only (0700/0600); with `encrypt` enabled they are AES-256-GCM
  records keyed by `~/.screendoc/key` (0600). Losing the key file loses
  the data — that is intended.
- **Network:** `serve` binds 127.0.0.1 only. `render --web` output is a
  local file; distribution is the user's action.
- **Intended trust boundary:** a single user's terminal recordings on a
  single host. screendoc is not a multi-tenant service; there is no
  remote attacker surface beyond the usual "user can run commands".

## Bug bounty

No formal bounty program at this stage. Responsible disclosure is
appreciated; credited in the advisory when the reporter consents.

## Verifying release artifacts

Release artifacts are published with SHA-256 checksums
(`<artifact>.sha256`, combined `SHA256SUMS`). When the release signing
secret is configured, every artifact and the checksum manifest is also
detached-signed (`<artifact>.asc`) with the release signing key:

- **Key (ASCII armor):** [`PGP-KEY.asc`](PGP-KEY.asc) — also published
  as a release asset.
- **Fingerprint:** `ADE1CFE02922679A2DE57563AE472669ED78D899`
  (RSA-4096, expires 2036-11).

Verify a download:

```sh
gpg --import PGP-KEY.asc
gpg --verify screendoc-aarch64-linux-musl.tar.gz.asc \
             screendoc-aarch64-linux-musl.tar.gz
sha256sum -c SHA256SUMS
```

The public key must be obtained from a trusted channel (this repository,
or the release page it points to); a key swapped into a compromised
release would defeat the check — that is why the fingerprint is also
published here and in the README.
