# Releasing Rowhouse

Pushing a tag `v*` runs `.github/workflows/release.yml`, which builds installers for
macOS (Apple silicon and Intel, built in parallel), Windows and Linux and attaches
them to a GitHub Release.
`rowhouse-mcp` is built and bundled automatically (`scripts/sidecar.mjs`).

```bash
node scripts/version.mjs 0.7.0      # Cargo.toml, tauri.conf.json, package.json
git commit -am "chore(release): v0.7.0"
git tag v0.7.0 && git push origin main v0.7.0
```

The workflow refuses to build a tag that doesn't match those three versions
(`node scripts/version.mjs --check v0.7.0`).

Everything below is optional — without it you still get working, **unsigned** builds.

## 1. Auto-update (recommended, free)

```bash
node scripts/setup-updater.mjs
```

This creates `~/.tauri/rowhouse-updater.key`, writes the public key and the
`latest.json` endpoint into `tauri.conf.json` and enables updater artifacts.
Commit the config change, then add two repository secrets:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | contents of `~/.tauri/rowhouse-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password you chose |

From the next release on, the app checks GitHub Releases at start-up (and every
6 hours) and offers **Install & restart**. Builds made before this step don't
update themselves.

## 2. macOS signing + notarization (Apple Developer Program, $99/yr)

Without it, users must right-click → Open the first time (Gatekeeper warning).

1. In Xcode / developer.apple.com create a **Developer ID Application** certificate,
   export it from Keychain as `.p12`.
2. Create an app-specific password for your Apple ID (appleid.apple.com).
3. Add secrets:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | `base64 -i cert.p12` output |
| `APPLE_CERTIFICATE_PASSWORD` | the .p12 export password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | the app-specific password |
| `APPLE_TEAM_ID` | your 10-character team id |

## 3. Windows signing (code-signing certificate)

Without it, SmartScreen shows "Windows protected your PC" until the app builds reputation.

1. Buy an OV/EV code-signing certificate (or use Azure Trusted Signing — then replace
   the import step with a `signCommand`, see the Tauri docs).
2. Export it as `.pfx` and add secrets:

| Secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE` | `base64 cert.pfx` output |
| `WINDOWS_CERTIFICATE_PASSWORD` | the .pfx password |

The workflow imports it and sets `certificateThumbprint` for the Windows build only.

## Licenses (Pro)

Pro features (scheduled queries, AI write requests over MCP) check for a license
**only in builds that embed a public key**. Anyone building from source gets a
community build with everything unlocked — that's intended (open core).

1. Once: `node scripts/license.mjs keygen` → creates `~/.rowhouse-license/private.pem`
   (back it up; never commit it) and prints the public key.
2. Add it as the repository secret `ROWHOUSE_LICENSE_PUBKEY`. The release workflow
   bakes it in, plus the build time (used for "updates until").
3. For each sale: `node scripts/license.mjs issue --email buyer@example.com --plan pro --years 1`
   and send the `RH1-…` key. Customers paste it in Settings → License. Keys are
   verified offline; after `updatesUntil` the license keeps working on the builds
   released before that date.
4. `node scripts/license.mjs verify RH1-…` checks a key.

## Homebrew & winget

After a release is published: `node scripts/packaging.mjs 0.4.0` downloads the
macOS `.dmg` and Windows `-setup.exe`, computes their SHA-256 and writes
`packaging/out/rowhouse.rb` (copy to your tap repo `giankbr/homebrew-tap/Casks/`)
and `packaging/out/Novagen.Rowhouse.yaml` (submit to `microsoft/winget-pkgs`, e.g.
with `wingetcreate submit`). Templates live in `packaging/`.
