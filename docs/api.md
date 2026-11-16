# JSON API (for the iOS and Android apps)

Everything lives under `/api/v1`, speaks JSON, and answers errors as
`{"error": "<Arabic message>"}`. It uses the same services and rules as the web
pages, so each user only ever sees and changes their own rows.

## Sign-in

1. `POST /api/v1/auth/code` `{"email": "..."}` → `204`. Emails a 6-digit code
   (same limits as the web: 10 minutes, single use, 5 tries, 5 codes an hour).
2. `POST /api/v1/auth/verify` `{"email": "...", "code": "123456"}` →
   `{"token": "...", "user": {"id", "email", "name"}}`. Arabic digits are fine.
3. Send `Authorization: Bearer <token>` on every other call. The app keeps the
   token in the phone's secure storage (Keychain / Keystore). Tokens are
   ordinary sessions: they renew on use and end after 365 unused days or on sign-out, and removed from `ALLOWED_EMAILS` means signed out.
4. `POST /api/v1/auth/logout` (with the token) → `204`.

`GET /api/v1/me` → the signed-in user. A missing or expired token gives `401`.
`DELETE /api/v1/me` → `204`: deletes the account and everything it owns (rows,
phones, site, drafts), module by module. App Store rules require this in the app.

`GET /api/v1/auth/options` → `{"google": bool}`: which sign-in buttons to show.

### Google (no extra Google key: it reuses the web's OAuth client)

1. The app makes a random `verifier` and opens
   `/auth/google?app=<hex sha256 of verifier>` in the system sign-in sheet.
2. After Google, the server sends the sheet to `aqrgo://auth?code=<one-time code>`
   (or `aqrgo://auth?error=google|not_allowed`). The code lives 2 minutes, once.
3. `POST /api/v1/auth/google` `{"code", "verifier"}` → `{"token", "user"}`. A code
   caught by another app is useless without the verifier (PKCE-style).

### Apple

`POST /api/v1/auth/apple` `{"identity_token", "name"}` → `{"token", "user"}`. The
server checks Apple's signature (keys from appleid.apple.com, cached), issuer,
audience `com.aqrgo.app` and expiry. Apple sends the name only on the first
sign-in, from the device.

### App Store review login

With the secrets `APP_REVIEW_EMAIL` and `APP_REVIEW_CODE` (10+ digits) set, that
one email signs in with that fixed code and gets no email. Give both to Apple in
App Store Connect → App Review Information.

## Telegram and WhatsApp bots

`GET /api/v1/channels` → `{"telegram": {"available", "linked"}, "whatsapp": {...}}`.
`available` is false when this server runs no bot for that channel.

`POST /api/v1/channels/{telegram|whatsapp}/link` → `{"url"}`: a one-time link
(`https://t.me/<bot>?start=<token>`, or `https://wa.me/<number>?text=ربط <token>`)
that the app opens; the bot ties that chat to the account. Same tokens as the
web account page. `DELETE /api/v1/channels/{channel}` → `204` unlinks. An
unknown or unavailable channel answers `404`.

## Listings

| Call | Answer |
| --- | --- |
| `GET /api/v1/listings?q=&status=` | `{"listings": [row, ...]}`, newest first. `q` searches every column (Arabic or Western digits); `status` is `available`, `reserved` or `sold`. |
| `GET /api/v1/listings/meta` | Form fields with Arabic labels, quick offer types, statuses. |
| `POST /api/v1/listings` | `201` + row. Body: any of the field keys, `listed_on` (`YYYY-MM-DD`, default today in Riyadh), `status`. |
| `GET /api/v1/listings/{id}` | row, or `404` (also for another user's row). |
| `PUT /api/v1/listings/{id}` | row. Replaces all fields: send the whole row. |
| `DELETE /api/v1/listings/{id}` | `204`. |
| `POST /api/v1/listings/extract` `{"text"}` | `{"listings": [draft, ...]}`: the Telegram bot's AI reads pasted text. Saves nothing; the app fills its form with a draft for review. `503` when the AI isn't set up, `422` for empty or over 4000 characters. |

A row carries every stored field plus `number` (running number in Arabic-Indic
digits), `status_label`, `date_hijri` and `date_gregorian`, ready to show.
Invalid input (no offer type, location or plan number; a bad date or status)
answers `422` with the Arabic message to show.

## Push notifications

| Call | Answer |
| --- | --- |
| `POST /api/v1/push/devices` `{"token": "ExponentPushToken[...]", "platform": "ios" \| "android"}` | `204`. A phone belongs to whoever signed in on it last. |
| `DELETE /api/v1/push/devices` `{"token": "..."}` | `204`. Sent by the app on sign-out. |

New rows that arrive from Telegram (and WhatsApp, once that channel lands)
notify every phone of their owner: title «عرض جديد من تيليجرام», body
«نوع العرض · الموقع», data `{"listing_id": N}`. Rows typed in on the web or the
app don't notify. Sending runs in a background job (`push.notify`) through
Expo's push service; phones that uninstalled the app are forgotten.
