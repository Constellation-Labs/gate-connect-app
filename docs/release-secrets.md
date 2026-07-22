# Release secrets checklist

GitHub Actions secrets consumed by `.github/workflows/release.yml` (the
tag-triggered `Release` job) and `.github/workflows/release-notes-slack.yml`
(posts release notes to Slack when a release is published). Add them under
**Settings → Secrets and variables → Actions → New repository secret**.

The macOS job (`macos-latest`) is the only one that code-signs and notarizes;
the Linux/Windows jobs ignore the `APPLE_*` secrets. The Tauri *updater*
signature (`TAURI_SIGNING_*`) is separate from Apple signing and is applied on
every platform.

## Checklist

| Secret | Required | Used for |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | yes | Signs the auto-updater artifacts; must match the `pubkey` in `tauri.conf.json`. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | yes | Password for the updater signing key. |
| `APPLE_CERTIFICATE` | macOS | base64 of the Developer ID Application `.p12`; the bundler imports it into a temp keychain. |
| `APPLE_CERTIFICATE_PASSWORD` | macOS | Password for the `.p12`. |
| `APPLE_SIGNING_IDENTITY` | macOS | Cert identity name, e.g. `Developer ID Application: <Name> (<TEAMID>)`. Set explicitly because `tauri.conf.json` pins `signingIdentity` to `"-"` (ad-hoc) for local builds, which would otherwise win. |
| `APPLE_API_KEY_P8` | macOS | base64 of the App Store Connect API key `.p8`; decoded to a file the bundler reads for notarization. |
| `APPLE_API_KEY` | macOS | App Store Connect **Key ID** (the `XXXX` in `AuthKey_XXXX.p8`). |
| `APPLE_API_ISSUER` | macOS | App Store Connect **Issuer ID** (UUID, shown above the keys table). |
| `VITE_POSTHOG_KEY` | optional | Inlined into the frontend by Vite. Absent ⇒ analytics is a no-op; build still succeeds. |
| `SLACK_WEBHOOK_URL` | yes | Slack incoming-webhook URL bound to `#gate-release-notes`; `release-notes-slack.yml` posts the release notes there when a stable (non-prerelease, non-`-rc`) release is published. |
| `GITHUB_TOKEN` | n/a | Auto-provided by Actions; no setup needed. |

## Producing the values

Run on Linux or macOS. On Linux (OpenSSL 3.x) keep the `-legacy` flag when
reading Apple's `.p12`; on macOS (LibreSSL) drop it.

```bash
# APPLE_CERTIFICATE  — base64 the Developer ID Application .p12 (single line)
openssl base64 -A -in GateAICert.p12

# APPLE_API_KEY_P8   — base64 the App Store Connect .p8 (single line)
openssl base64 -A -in AuthKey_XXXX.p8

# APPLE_SIGNING_IDENTITY — the certificate's Common Name
openssl pkcs12 -in GateAICert.p12 -nokeys -clcerts -passin pass:'P12_PASSWORD' -legacy 2>/dev/null \
  | openssl x509 -noout -subject -nameopt multiline,utf8 2>/dev/null \
  | sed -n 's/ *commonName *= *//p'
# → Developer ID Application: <Name> (<TEAMID>)
#   On macOS you can instead use: security find-identity -v -p codesigning
```

- `SLACK_WEBHOOK_URL`: [api.slack.com/apps](https://api.slack.com/apps) →
  create (or reuse) an app in the workspace → **Incoming Webhooks** → activate →
  **Add New Webhook to Workspace** → pick `#gate-release-notes` → copy the
  `https://hooks.slack.com/services/…` URL. The webhook is channel-bound: to
  change channels, create a new webhook and update the secret.
- `APPLE_API_KEY` is the Key ID embedded in the `.p8` filename (`AuthKey_<KEYID>.p8`).
- `APPLE_API_ISSUER` is in App Store Connect → **Users and Access → Integrations
  → App Store Connect API**, above the keys table.
- The `certSigningRequest` (CSR) is only used to generate the cert in the Apple
  Developer portal; it is **not** a CI secret.

## Notes

- The signing cert must be a **Developer ID Application** cert (for distribution
  outside the App Store). `security find-identity` / the cert CN will say so.
- Tauri enables the hardened runtime automatically when signing with a real
  identity, which notarization requires. No entitlements file is needed unless
  notarization later flags a missing one.
- macOS signing/notarization cannot run on a Linux runner — it needs Apple
  tooling (`codesign`, `notarytool`). Generating the secret *values* on Linux is
  fine; the signing step itself stays on `macos-latest`.