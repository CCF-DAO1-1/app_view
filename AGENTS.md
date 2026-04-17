# AGENTS.md — DAO App View

## What this is
A Rust binary (`dao`) that serves a REST API for a DAO system built on CKB (Nervos) and AT Protocol (Bluesky). It runs an Axum web server, a Postgres-backed data layer, background cron jobs, and an AT Protocol firehose relayer subscription.

## Quick commands

```bash
# Build release binary
cargo build --release

# Run (requires a running Postgres instance and external CKB/ATProto services)
cargo run -- --db-url "postgres://..." --ckb-url "..." --indexer-bind-url "..." --indexer-did-url "..." --indexer-vote-url "..." --indexer-dao-url "..." --relayer "..." --pds "..."
```

## Architecture & entrypoints
- **Binary entrypoint**: `src/main.rs`
  - Parses CLI args with `clap`, initializes logging (`common_x::log::init_log_filter`), connects to Postgres, starts the AT Protocol relayer reconnect loop, starts the cron scheduler, and binds the Axum HTTP router.
- **Library surface**: `src/lib.rs` exposes modules and the shared `AppView` struct (database pool + CKB client + indexer URLs + PDS/relayer config).
- **API handlers**: `src/api/` — Axum route handlers for proposals, votes, tasks, meetings, timeline, likes, replies.
- **Data models / schema init**: `src/lexicon/` — domain models (Proposal, Vote, Task, Meeting, etc.). **There is no `migrations/` directory**; each model provides an `init(&db)` async method that creates its own tables on startup.
- **Background jobs**: `src/scheduler/` — `tokio-cron-scheduler` jobs that run every few seconds to build voter lists, check CKB transaction confirmations, and finalize votes.
- **AT Protocol ingestion**: `src/relayer/` — subscribes to a Repo firehose, parses records, and feeds them into the app state.
- **CKB integration**: `src/ckb.rs`, `src/indexer_*.rs`, `src/smt.rs` — blockchain address parsing, transaction building, and indexer HTTP clients.
- **Molecule schemas**: `molecules/vote.mol` — CKB molecule serialization schema. There is **no `build.rs`**; molecule code appears to be pre-generated or handled externally.

## Build & lint configuration
- **Edition**: 2024 (Rust 1.85+ likely required).
- **No `Cargo.lock` in repo**: it is gitignored. Fresh builds will resolve dependencies at build time. The Dockerfile does not vendor a lockfile either.
- **Lint rules live in `Cargo.toml`**:
  - `missing_const_for_fn = "warn"`
  - `unsafe_code = "forbid"`
  - `unused_extern_crates = "warn"`
- **Release profile**: aggressive optimization (`lto = "fat"`, `opt-level = 3`, `codegen-units = 1`). Release builds can be slow.
- **No `rustfmt.toml`, `clippy.toml`, or `.cargo/config`** present.

## Docker
- **Dockerfile**: multi-stage build using `m.daocloud.io/docker.io/rust:slim-bullseye`.
- The final image copies `/build/target/release/dao` to `/usr/bin/dao` and runs as user `dao`.
- **Note**: the current `CMD` uses shell-style variable expansion inside a JSON array (`CMD ["dao", "--db-url $DB_URL", ...]`), which Docker will **not** expand. If changing the image entrypoint behavior, prefer an entrypoint script or `ENTRYPOINT` + `CMD` shell form.

## CI / automation
- `.github/workflows/docker.yml`: manual trigger only (`workflow_dispatch`).
  - Pushes to `ghcr.io/${{ github.repository }}`.
  - `main` branch → `latest` tag.
  - `v*` git tags → corresponding image tag.

## Testing
- **No `tests/` directory or `build.rs`**.
- There do not appear to be dedicated integration tests in the repo. Verification is typically done by building the release binary and running the service locally against a full stack (Postgres + CKB indexers + AT Protocol relay/PDS).

## Dependencies of note
- `common_x` — external crate providing shared `restful::axum` wrappers and log initialization. It is not in this repo.
- `ckb-sdk`, `ckb-types`, `ckb-hash`, `sparse-merkle-tree` — Nervos/CKB ecosystem crates.
- `atrium-api`, `atrium-repo` — AT Protocol (Bluesky) Rust SDK.
- `sqlx` + `sea-query` / `sea-query-sqlx` — async Postgres query building and execution.
- `utoipa` / `utoipa-scalar` — OpenAPI documentation generation and Scalar UI.
- `tokio-cron-scheduler` — cron-like background task scheduling.

## Operational gotchas
- The app **requires a live Postgres database** and multiple external HTTP endpoints on startup. It will fail to start if any required `--*url` argument is missing.
- Database tables are created automatically by `lexicon` `init()` calls in `main.rs`. Do not look for `sqlx migrate` or Diesel migrations.
- The relayer connection runs in a dedicated Tokio task with an infinite reconnect loop; errors are logged but do not crash the process.
