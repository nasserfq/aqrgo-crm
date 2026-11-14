# AQRGO CRM: spec

Agreed with Nasser on 2026-11-13 (project thread "Build real estate CRM with Telegram leads").

## Goal

A simple Arabic, right-to-left real estate CRM that reproduces the office's Word table, where leads can be added from a Telegram bot.

## Table (one row = one property)

Columns, right to left, in this order (from the Word table), plus the columns requested afterwards on WhatsApp:

م · نوع العرض · رقم المخطط · رقم القطعة · المساحة · الواجهات والشوارع · الموقع · السعر · بيانات المشتري · بيانات البائع · بيانات الوسيط · اليوم والتاريخ · الحالة

- م is each user's own running number (1, 2, 3…), shown in Arabic-Indic digits like the Word table.
- الحالة: متاح / محجوز / مباع. The buyer is filled in later.
- نوع العرض: quick choices plus free text.
- اليوم والتاريخ: weekday + Hijri (Umm al-Qura) + Gregorian, defaults to today (Riyadh time), editable.
- Search across all columns (Arabic or Western digits), filter by status, export to Excel (CSV), print in landscape like the Word page.

## Users and sign-in

- Single users, no teams yet. Anyone can sign up; each user sees and edits only their own rows. Optional `ALLOWED_EMAILS` restricts sign-up.
- Sign in with Google, or with a 6-digit code sent by email. Codes expire after 10 minutes, are single use, lock after 5 wrong tries, max 5 requests per hour per email.
- Emails go through AWS SES, sent by a background job.

## Telegram

- Each user links exactly one Telegram account (one-time deep link from the account page).
- Free text (or a forwarded WhatsApp message) is read by AI (Amazon Bedrock, Amazon Nova by default, model id configurable; not Claude) and turned into one draft per property.
- Every draft is shown as a preview with Save / Edit / Cancel. Nothing is saved without confirmation. Edit lets the user change any field.
- Step-by-step entry (/new) asks each field with a skip button. It is the fallback when AI is off or fails.
- /last shows the latest rows. Telegram updates are processed by background jobs.

## Architecture

- Rust + Axum modular monolith (Cargo workspace). Modules: `listings`, `auth`, `telegram`, `ai`, `jobs`, `mailer`, `common`, wired by `app`. Modules talk through public APIs and each owns its own Postgres schema and migrations.
- Server-rendered pages (Askama + a little htmx), plus a JSON API under `/api/v1` with bearer tokens for the iOS and Android apps (see `docs/api.md`).
- PostgreSQL, self-hosted next to the app (not Supabase), with daily backups.
- Background jobs: Postgres-backed queue with retries.
- No Redis for now (Postgres covers current needs).
- Built and deployed by GitHub Actions (never compiled on the server): test, build Docker image to GHCR, SSH to the server, `docker compose up`. The server's shared kamal-proxy provides HTTPS for aqrgo.com and www.aqrgo.com.
- Built test-first (TDD) at agreed seams.
