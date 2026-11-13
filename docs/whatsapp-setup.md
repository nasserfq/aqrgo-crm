# WhatsApp setup (Meta WhatsApp Cloud API, direct)

AQRGO talks to Meta's WhatsApp Cloud API itself, with no reseller like Twilio. Replies sent within 24 hours of a user's message are free. The bot only replies, so normal use costs nothing.

Do the steps in this order. Meta renames menus often, so if a name below has changed, look for the closest match.

## 1. Before you start

- **A phone number that is not on WhatsApp.** It can be a new SIM or a landline. It must be able to receive an SMS or a voice call for the code. If the number is on the WhatsApp app now, delete that WhatsApp account first. Once it is on the Cloud API, it can't be used in the WhatsApp app.
- **A Meta business portfolio** at business.facebook.com.
- **Business verification** (Business settings → Security Center → Start verification). This is recommended. Without it Meta keeps low limits and may not show the display name. The bot only replies to people who write first, so it can work before verification is done.

## 2. Create the app

1. Go to developers.facebook.com → My Apps → Create app.
2. Pick the use case **Connect with customers through WhatsApp**, then link your business portfolio.
3. Open **WhatsApp → API Setup**. Meta gives you a free test number; ignore it.
4. Click **Add phone number**.
   - Enter the display name **AQRGO**. Meta reviews it.
   - Enter the number and confirm the code.
   - If Meta asks for a payment method, add one. Replies within 24 hours stay free.
5. Copy the new number's **Phone number ID**. You'll need it in step 4.

## 3. A permanent access token

The token on the API Setup page expires within a day, so make a permanent one:

1. Business settings → Users → **System users** → Add, and give it the Admin role.
2. Click **Assign assets** and give full control of:
   - the app
   - the WhatsApp account
3. Click **Generate token**.
   - Pick the app.
   - Set expiry to **Never**.
   - Tick `whatsapp_business_messaging` and `whatsapp_business_management`.
4. Copy the token. Meta shows it only once.

## 4. GitHub secrets (never paste these in chat)

Go to Repository → Settings → Secrets and variables → Actions → New repository secret, and add:

| Secret | Value |
|---|---|
| `WHATSAPP_TOKEN` | the system user token from step 3 |
| `WHATSAPP_PHONE_NUMBER_ID` | the Phone number ID from step 2 |
| `WHATSAPP_APP_SECRET` | App settings → Basic → App secret → Show |
| `WHATSAPP_VERIFY_TOKEN` | any long random string you make up, e.g. from a password manager |

WhatsApp stays off until all four exist. A half-finished setup never takes the site down. Then re-run the latest **Test, build and deploy** workflow (or merge any change). After the deploy, the log line `whatsapp ready` shows the number.

## 5. Point Meta at AQRGO (after the deploy)

1. In the app, open **WhatsApp → Configuration → Webhook → Edit** and fill in:
   - Callback URL: `https://aqrgo.com/whatsapp/webhook`
   - Verify token: the same value as `WHATSAPP_VERIFY_TOKEN`
2. Click **Verify and save**. Meta calls the site and AQRGO answers only if the token matches.
3. Under **Webhook fields**, subscribe to **messages**.
4. Set the app to **Live**, using the App mode switch at the top. Meta asks for a privacy policy URL first.

## 6. Link and test

1. On aqrgo.com, open My account (حسابي) → WhatsApp link (ربط واتساب) → Create link (إنشاء رابط الربط).
2. The link opens WhatsApp with `ربط <code>` already typed. Press Send.
3. Send or forward a lead. The bot replies with a preview and the buttons Save, Edit and Cancel (حفظ / تعديل / إلغاء).

## If messages don't arrive

- **Check the deploy log.** `whatsapp number lookup failed` means the token or the Phone number ID is wrong.
- **Check the webhook field.** In WhatsApp → Configuration, make sure **messages** is subscribed.
- **Subscribe the app to the WhatsApp account.** Meta does this automatically for numbers added in the dashboard. If it didn't, run this once, using the token:
  `curl -X POST "https://graph.facebook.com/v23.0/<WHATSAPP_BUSINESS_ACCOUNT_ID>/subscribed_apps" -H "Authorization: Bearer <token>"`
