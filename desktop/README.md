# AQRGO for Mac, Windows and Linux

A small Rust app ([Tauri 2](https://tauri.app)) that shows aqrgo.com in its own
window. Everything new on the website appears in the app without a new release.

- **Sign-in**: email code and Sign in with Apple run in the window. Google
  refuses embedded windows, so «الدخول بحساب Google» opens the default browser;
  after Google the browser opens `aqrgo-desktop://auth?code=…` and the app
  posts that one-time code, with a secret it kept (its verifier), to
  `/auth/app/session`, which sets the normal session cookie. Same idea as the
  phone app, on its own link so a phone app installed on a Mac can't catch it.
- **Links**: aqrgo.com and Apple's sign-in pages stay in the window. Anything
  else (WhatsApp, Telegram, maps, agent sites) opens in the browser.
- **Downloads** (Excel export) go to Downloads and the folder opens.
- **Offline**: the start page (`dist/index.html`) waits for a connection.

The site's own pieces live in `crates/auth` (`/auth/app/session`,
`/auth/google?client=desktop`, `/auth/apple`).

## Build and release

GitHub Actions (`.github/workflows/desktop.yml`) does everything:

- Pull requests build all three and attach them to the run (Artifacts).
- Every push to `main` that touches `desktop/` publishes the release
  `desktop-v<version>` with the installers. Bump `version` in
  `src-tauri/tauri.conf.json` (and `Cargo.toml`) for a new release.
- Downloads: <https://github.com/nasserfq/aqrgo-crm/releases/latest>

| | File |
|---|---|
| Mac (Apple Silicon and Intel) | `AQRGO_<v>_universal.dmg` |
| Windows | `AQRGO_<v>_x64-setup.exe` (or `.msi`) |
| Linux | `AQRGO_<v>_amd64.AppImage`, `.deb`, `.rpm` |

### Signing

Without signing the apps still run, but the first launch warns: on Mac,
System Settings → Privacy & Security → Open Anyway; on Windows, SmartScreen
→ More info → Run anyway.

Mac signing and notarization turn on by themselves when these GitHub
secrets exist:

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | Developer ID Application certificate + key, `.p12`, base64 on one line |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Name (PNW88M62LY)` |
| `APPLE_API_KEY` / `APPLE_API_ISSUER` / `APPLE_API_KEY_P8` | App Store Connect API key id, issuer id and the `.p8` text, for notarization |

## Run it locally

Needs Rust and Node 22 (on Linux also `libwebkit2gtk-4.1-dev`).

```bash
cd desktop
npm ci
npx tauri dev      # or: npx tauri build
cd src-tauri && cargo test
```

Debug builds can point at a local server for end-to-end testing:
`AQRGO_SITE=http://localhost:8080 npx tauri dev` (release builds ignore it).
