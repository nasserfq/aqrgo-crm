# AQRGO for iOS and Android

React Native app built with Expo (SDK 58) and Expo Router. Arabic, right to
left (forced natively), lavender design from the web app. It talks to the
server's JSON API (`docs/api.md`) at `extra.apiUrl` in `app.json`.

## What it does

- Sign in with the 6-digit code emailed to you (token kept in the Keychain / Keystore).
- The table as cards: search, status filter, pull to refresh.
- Add, edit and delete rows (same form groups and rules as the web).
- Push notifications for new leads that arrive from Telegram; tapping one opens the row.

## Work on it

```bash
cd mobile
npm ci
npx tsc --noEmit   # types
npx jest           # tests (whole app through Expo Router, against a fake server)
npx expo start     # then open in a development build
```

`src/app/` holds the screens (one file per route), `src/lib/` the API client,
session and notifications, `src/components/` the shared pieces. Install
packages with `npx expo install`, never plain `npm install`, so versions match the SDK.

## Builds and the stores (EAS)

Builds, signing and store uploads run on Expo's servers, not on our server and
not in GitHub Actions:

```bash
npx eas-cli@latest login
npx eas-cli@latest init                              # links the app, adds extra.eas.projectId (needed for push)
npx eas-cli@latest build --profile preview -p android   # an APK to install directly
npx eas-cli@latest build --profile production -p all    # store builds
npx eas-cli@latest submit -p ios                         # TestFlight / App Store
npx eas-cli@latest submit -p android                     # Play Console
```

Bundle id / package: `com.aqrgo.app`. Push notifications need the EAS project id
(from `eas init`) and, for iOS, an Apple push key, which `eas build` offers to create.
The server sends through Expo's push service; set `EXPO_ACCESS_TOKEN` in GitHub
secrets only if "enhanced push security" is turned on in the Expo account.
