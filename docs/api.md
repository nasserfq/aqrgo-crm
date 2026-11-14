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
   ordinary sessions: 30 days, and removed from `ALLOWED_EMAILS` means signed out.
4. `POST /api/v1/auth/logout` (with the token) → `204`.

`GET /api/v1/me` → the signed-in user. A missing or expired token gives `401`.

## Listings

| Call | Answer |
| --- | --- |
| `GET /api/v1/listings?q=&status=` | `{"listings": [row, ...]}`, newest first. `q` searches every column (Arabic or Western digits); `status` is `available`, `reserved` or `sold`. |
| `GET /api/v1/listings/meta` | Form fields with Arabic labels, quick offer types, statuses. |
| `POST /api/v1/listings` | `201` + row. Body: any of the field keys, `listed_on` (`YYYY-MM-DD`, default today in Riyadh), `status`. |
| `GET /api/v1/listings/{id}` | row, or `404` (also for another user's row). |
| `PUT /api/v1/listings/{id}` | row. Replaces all fields: send the whole row. |
| `DELETE /api/v1/listings/{id}` | `204`. |

A row carries every stored field plus `number` (running number in Arabic-Indic
digits), `status_label`, `date_hijri` and `date_gregorian`, ready to show.
Invalid input (no offer type, location or plan number; a bad date or status)
answers `422` with the Arabic message to show.
