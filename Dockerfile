# syntax=docker/dockerfile:1.7
# Built by GitHub Actions; the server only pulls the finished image.

FROM rust:1-bookworm AS build
WORKDIR /src
RUN apt-get update \
 && apt-get install -y --no-install-recommends cmake clang \
 && apt-get clean
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p app \
 && cp target/release/aqrgo-crm /aqrgo-crm

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && apt-get clean \
 && useradd --system --uid 10001 app
COPY --from=build /aqrgo-crm /usr/local/bin/aqrgo-crm
USER app
ENV PORT=8080 RUST_LOG=info,sqlx=warn
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=20s CMD curl -fsS http://localhost:8080/healthz || exit 1
CMD ["aqrgo-crm"]
