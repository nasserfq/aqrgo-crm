# AQRGO CRM

Arabic, right-to-left real estate CRM. Each user keeps their own property table (the office's Word table, plus بيانات الوسيط, اليوم والتاريخ and الحالة), and can add rows from the website or by sending a message to a Telegram bot, where Amazon Bedrock reads the message and fills in the columns for the user to confirm.

See [docs/spec.md](docs/spec.md) for what was agreed.

## Layout (modular monolith)

| Crate | Owns | Does |
|---|---|---|
| `listings` | schema `listings` | Property rows, search, Excel export, table pages |
| `auth` | schema `auth` | Google sign-in, 6-digit email codes, sessions, Telegram linking |
| `telegram` | schema `telegram` | Bot conversation, drafts, webhook |
| `ai` | — | Amazon Bedrock lead extraction |
| `jobs` | schema `jobs` | Postgres job queue + worker |
| `mailer` | — | Sends email via AWS SES from a job |
| `common` | — | Shared types, Hijri dates, Arabic digits, layout |
| `app` | — | Config and wiring (the binary) |

Modules call each other only through their public APIs. Each runs its own migrations in its own Postgres schema at startup.

## Run locally

```bash
docker run -d --name crm-db -e POSTGRES_PASSWORD=postgres -p 5433:5432 postgres:17-alpine
DATABASE_URL=postgres://postgres:postgres@localhost:5433/postgres cargo run -p app
# open http://localhost:8080 — without AWS settings the login code is printed in the log
```

Tests need a Postgres where the user can create databases:

```bash
TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5433/postgres cargo test --workspace
```

## Deploy (GitHub Actions → your server)

Every push to `main` runs the tests, builds the Docker image on GitHub, pushes it to GitHub's container registry, then connects to the server over SSH and restarts it. Nothing is compiled on the server.

**One-time server setup** (any Linux server with Docker):

1. Point `crm.aqrgo.com` (A record) at the server's IP. Open ports 80 and 443.
2. `sudo mkdir -p /opt/aqrgo-crm && sudo chown $USER /opt/aqrgo-crm`
3. Copy `deploy/.env.example` to `/opt/aqrgo-crm/.env` and fill it in (see below).

**GitHub repository settings → Secrets and variables → Actions:**

| Secret | Value |
|---|---|
| `SSH_HOST` | Server IP or hostname |
| `SSH_USER` | User that can run `docker` |
| `SSH_KEY` | Private SSH key for that user |
| `SSH_PORT` | Optional, default 22 |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | AWS user for SES email. Each deploy writes them into the server `.env` |
| `AWS_BEARER_TOKEN_BEDROCK` | Optional Bedrock API key. Each deploy writes it into the server `.env` |

Optional variable `DEPLOY_DIR` (default `/opt/aqrgo-crm`). Create an environment called `production` if you want to approve each deploy.

**Settings in `.env`:**

- **AWS**: an IAM user with `ses:SendEmail` and `bedrock:InvokeModel`. Instead of giving that user Bedrock access, you can paste a Bedrock API key into `AWS_BEARER_TOKEN_BEDROCK`. Its region must match `AWS_REGION`. In SES, verify the `aqrgo.com` domain (it gives you DNS records) and request production access. In Bedrock, enable access to the Amazon Nova model in your region.
- **Google sign-in** (optional): Google Cloud Console → APIs & Services → Credentials → OAuth client (Web). Redirect URI: `https://crm.aqrgo.com/auth/google/callback`.
- **Telegram**: create a bot with @BotFather, paste the token, and set `TELEGRAM_WEBHOOK_SECRET` to a random string. The app registers its webhook on startup.

Backups: a daily `pg_dump` lands in `/opt/aqrgo-crm/backups` (14 days kept). Copy them off the server too.
