# Releasing Rowhouse

Pushing a tag `v*` runs `.github/workflows/release.yml`, which builds installers for
macOS (universal), Windows and Linux and attaches them to a GitHub Release.
`rowhouse-mcp` is built and bundled automatically (`scripts/sidecar.mjs`).

```bash
# bump the version in apps/desktop/src-tauri/tauri.conf.json and apps/desktop/package.json
git tag v0.2.0 && git push origin v0.2.0
```

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
