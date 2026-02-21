# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Working Style

Prefer direct action over planning. Do NOT enter plan mode or spawn sub-agents unless explicitly asked. When the user gives a clear task, execute it immediately.

## Project Overview

TypeDB is a graph database written in Rust with a declarative query language (TypeQL). It provides gRPC and HTTP APIs, MVCC-based storage on RocksDB, and a sophisticated type system for entities, relations, and attributes.

## Tech Stack

This is primarily a Rust project. Use idiomatic Rust patterns. The build system is Bazel, but Cargo is used for Rust development. The database engine uses RocksDB for storage, with a custom MVCC implementation and a write-ahead log (WAL) for durability.

## Build System

TypeDB uses **Bazel** as the primary build system and **Cargo** as a secondary build system for Rust development.

**Cargo.toml files are auto-generated** by the TypeDB Cargo sync tool. Do not modify them directly. The source of truth for build configuration is the Bazel BUILD files.

### Common Commands

```bash
# Build with Cargo
cargo build

# Build with Bazel
bazel build //:typedb_server_bin

# Run all Cargo tests
cargo test

# Run a specific test suite
cargo test --test test_query
cargo test --test test_concept
cargo test --test test_connection
cargo test --test test_assembly
cargo test --test test_http_database

# Run a specific test by name within a suite
cargo test --test test_query -- test_name_pattern

# Run tests for a specific crate
cargo test -p storage
cargo test -p encoding

# Format code
cargo fmt

# Check formatting
cargo fmt -- --check

# Bazel test (primary CI)
bazel test //...
```

### Available Test Suites (root-level)

`test_assembly`, `test_connection`, `test_concept`, `test_query`, `test_debug`, `bench_iam`, `test_http_database`, `test_http_transaction`, `test_http_driver_concept`, `test_http_driver_http`, `test_http_driver_query`, `test_http_driver_user`, `test_http_driver_connection`, `test_http_debug`

## Architecture

### Layer Diagram

```
Server Layer        main.rs → server/ (gRPC via tonic, HTTP via axum, auth, config)
                         ↓
Database Layer      database/ (DatabaseManager, transactions, lifecycle)
                         ↓
Query Layer         query/ → compiler/ → ir/ → executor/
                         ↓
Concept Layer       concept/ (type system: entities, relations, attributes)
                         ↓
Storage Layer       storage/ (MVCC) → durability/ (WAL) → encoding/ (binary key layout)
                         ↓
                    RocksDB (5 keyspace instances with tuned prefix extractors)
```

### Key Crates

- **server/** — gRPC (tonic) and HTTP (axum) services, authentication, CLI args, config
- **database/** — `DatabaseManager`, `Database<D>`, `Transaction` lifecycle. Generic over durability backend `D`
- **query/** — TypeQL query analysis, caching, orchestration between compiler and executor
- **compiler/** — TypeQL → IR compilation, query planning
- **ir/** — Intermediate representation (patterns, pipelines, translations)
- **executor/** — Query execution engine, match/read/write pipelines, batch processing
- **concept/** — Type system implementation, `ConceptAPI` trait
- **function/** — Built-in and user-defined function support
- **storage/** — MVCC storage with `ReadableSnapshot`/`WritableSnapshot` traits, isolation manager
- **durability/** — Write-ahead log (WAL), crash recovery. RocksDB's own WAL is disabled
- **encoding/** — Binary key encoding, keyspace definitions (`EncodingKeyspace` enum with 5 variants)
- **system/** — System database (internal metadata)
- **user/** — User management and authentication
- **resource/** — Constants, config values, profiling support
- **answer/** — Query result representation
- **common/** — 10 foundation sub-crates: `error`, `logger`, `cache`, `concurrency`, `bytes`, `lending_iterator`, `primitive`, `iterator`, `options`, `structural_equality`

### Key Design Patterns

**Generics over durability:** Core types like `MVCCStorage<D>`, `Database<D>`, and transactions are generic over a `DurabilityClient` trait, decoupling the engine from the specific WAL implementation.

**MVCC key format:** `[USER_KEY][SEQUENCE_NUMBER_INVERTED (8 bytes)][OPERATION (1 byte)]` — sequence numbers are bitwise-inverted for efficient prefix seeks; operation byte is 0x0=Insert, 0x1=Delete.

**5 separate RocksDB keyspaces** with different prefix lengths (11–25 bytes) optimized for different data patterns (schema, links, attributes, has-edges).

**Error handling:** Uses a `typedb_error!` macro (defined in `common/error/error.rs`) that generates structured error types implementing the `TypeDBError` trait with component, code, and description.

**Query pipeline:** TypeQL string → `query/analyse.rs` → `compiler/` (IR compilation) → `ir/` (intermediate representation) → `executor/` (pipeline execution) → `answer/` (result sets). Query plans are cached in `query/query_cache.rs`.

## Debugging Rules

When debugging, NEVER guess at root causes. Always verify hypotheses with evidence (logs, prints, git bisect, minimal repros) before proposing fixes. If a fix doesn't work, do NOT try another guess — step back and gather more data.

When fixing bugs, prefer the SIMPLEST approach first. If old code/docs need to go, just delete them — don't add status headers, wrappers, or migration layers unless explicitly asked.

## Git Workflow

When committing, be aware that git hooks may cause interactive prompts. If a commit hangs or fails due to hooks, immediately try `git commit --no-verify` rather than attempting multiple workarounds.

## Code Conventions

- Rust edition 2021
- All crates use `#![deny(unused_must_use)]` and `#![deny(elided_lifetimes_in_paths)]`
- rustfmt config: `max_width = 120`, `imports_granularity = "Crate"`, `group_imports = "StdExternalCrate"`, `use_small_heuristics = "Max"`
- MPL 2.0 license header required on all source files
- Conventional commits: `type(scope): message` (feat, fix, docs, refactor, test, chore, perf)

## Workspace Structure

43 crates in the workspace. Key sub-crate paths: `database/tools`, `durability/tests/crash/streamer`, `durability/tests/crash/recoverer`, `durability/tests/common`, `encoding/tests`, `concept/tests`, `storage/tests`, `tests/behaviour/steps`, `tests/behaviour/steps/params`, `tests/behaviour/service/http/http_steps`.

## Test Infrastructure

BDD-style tests live in `tests/behaviour/` with shared step definitions in `tests/behaviour/steps/`. Test utilities: `util/test/` (test_utils crate), `storage/tests/test_utils_storage/`, `encoding/tests/`. The `test_keyspace_set!` macro in test_utils_storage creates mock keyspace configurations for storage-level tests.
