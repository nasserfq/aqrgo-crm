# AQRGO CRM

Arabic RTL real estate CRM in Rust (Axum modular monolith). Read `docs/spec.md` first, then `README.md`.

- Modules (`crates/*`) talk only through each other's public APIs and each owns its own Postgres schema and migrations. `app` only wires them together.
- Build test-first at the module seams. Tests need `TEST_DATABASE_URL` (a Postgres where the user can create databases).
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` must pass; CI runs the same.
- Never compile on the server: GitHub Actions builds the image and deploys.
- UI text is Arabic and right-to-left.

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues (via the `gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

The default five labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## TypeSafe (Jev)

Use the `typesafe-ai` skill (plugin `typesafe@typesafe-ai`, enabled in `.claude/settings.json`) for any work on Jev decisions in lead extraction. Bedrock extracts the row fields; Jev answers typed judgments (is it a lead, offer type, status, duplicate) and code makes the decision. Leads are Arabic, so test questions on real Arabic messages before relying on them.
