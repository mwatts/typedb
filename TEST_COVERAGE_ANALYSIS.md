# TypeDB Test Coverage Analysis

## Overview

The TypeDB codebase contains approximately **525 Rust source files** across **40+ Cargo crates**, with **~136 test files** containing roughly **283 `#[test]` functions**, plus **91 behaviour (BDD/Gherkin) test runner files**. Testing follows a multi-layered strategy: inline unit tests, module-level integration tests, and top-level BDD behaviour tests.

## Current Coverage by Module

| Module | Source Files | Test Files | Inline `#[test]` | Coverage Level |
|---|---|---|---|---|
| **executor** | 55 | 12 | 77 | Good |
| **storage** | 28 | 7 | 45 | Good |
| **database** | 9 | 2 | 32 | Good |
| **durability** | 5 | 5 | — | Good |
| **encoding** | 47 | 4 | 29 | Moderate |
| **concept** | 41 | 4 | 25 | Moderate |
| **compiler** | 56 | 1 | 25 | Low |
| **ir** | 30 | 3 | 15 | Low |
| **query** | 12 | 3 | 4 | Low |
| **function** | 4 | 1 | 1 | Low |
| **server** | 53 | 0 | 3 | **Critical gap** |
| **diagnostics** | 9 | 0 | 0 | **No tests** |
| **answer** | 3 | 0 | 0 | **No tests** |
| **user** | 4 | 0 | 0 | **No tests** |
| **resource** | 5 | 0 | 0 | **No tests** |
| **common/** (10 sub-modules) | 24 | 0 | 11 | **Mostly untested** |

## Proposed Areas for Test Improvement

### 1. Server Module — Authentication & Authorization (P0 — Critical)

The `server/` module has **53 source files** but only **3 test functions** (all in `parameters/config.rs`). This is the most significant gap in the codebase.

**What's untested:**
- `authentication/token_manager.rs` — JWT token creation, validation, expiration, cleanup, and secret key generation. These are security-critical code paths.
- `authentication/credential_verifier.rs` — Password verification against stored credentials.
- `state.rs` — Server initialization, database creation/deletion, user management (create/update/delete), permission checks, and server ID handling.

**Recommended tests:**
- Unit tests for token lifecycle (create → validate → expire → cleanup)
- Tests for invalid/expired/malformed tokens
- Tests for credential verification with correct and incorrect passwords
- Tests for permission checks (authorized vs. unauthorized access)

### 2. Server Module — gRPC & HTTP Service Layers (P0 — Critical)

**What's untested:**
- `service/grpc/typedb_service.rs` — Connection open/close, database CRUD, user management, transaction creation.
- `service/http/typedb_service.rs` — All REST endpoints, routing, CORS configuration.
- `service/grpc/error.rs` and `service/http/error.rs` — Error-to-protocol conversion.
- `service/grpc/encryption.rs` and `service/http/encryption.rs` — TLS configuration and certificate loading.
- `service/http/message/` (10+ files) — Request/response serialization and deserialization.
- `service/transaction_service.rs` — Transaction lifecycle, query execution, timeout handling.
- `service/export_service.rs` and import/migration services.

**Recommended tests:**
- Request parsing and response building tests for both gRPC and HTTP
- Error conversion tests (internal errors → proper protocol error codes)
- TLS configuration tests (valid certs, missing certs, invalid certs)
- Message serialization round-trip tests
- Transaction lifecycle tests via the service layer

### 3. Compiler Module — Deeper Unit Test Coverage (P1 — High)

The compiler has **56 source files** but only **1 test file** (`transformation.rs`) with 25 tests. The compilation pipeline is core to query correctness.

**What's untested:**
- Annotation processing and type inference (`annotation/` subdirectory)
- Query optimization passes
- Query plan generation (`executable/match_/planner/plan.rs` — 2,321 lines)
- Constraint vertex planning (`executable/match_/planner/vertex/constraint.rs` — 1,316 lines)
- Expression compilation
- Edge cases in compilation (malformed inputs, unsupported constructs)

**Recommended tests:**
- Tests for each compilation stage in isolation
- Tests for annotation resolution and propagation
- Negative tests for invalid queries that should produce clear compilation errors
- Regression tests for known optimization edge cases

### 4. Common Libraries — `bytes`, `error`, `structural_equality`, `lending_iterator` (P1 — High)

The `common/` directory contains **10 sub-modules** (3,558 LOC total), but only **3 have any tests** (cache, lending_iterator/kmerge, logger/log_panic).

**Highest priority untested modules:**

- **`bytes/`** (555 LOC) — `ByteArray` and byte utility functions used across the entire codebase for key encoding. Incorrect byte handling could corrupt stored data.
- **`structural_equality/`** (432 LOC) — Used for query deduplication and caching. Incorrect equality checks could return wrong query results.
- **`error/`** (476 LOC) — Error construction macros and TypeQL error wrapping.
- **`lending_iterator/`** — Only `kmerge.rs` has tests. The main `lending_iterator.rs` (343 LOC) and `adaptors.rs` (503 LOC) have no tests despite being used extensively throughout the query engine.

**Recommended tests:**
- `ByteArray`: construction, comparison, slicing, endianness conversions
- `structural_equality`: equality and inequality across different TypeQL patterns
- `lending_iterator` adaptors: map, filter, chain, flat_map, take, skip — with edge cases (empty iterators, single elements)

### 5. Concept Module — Type Validation (P1 — High)

The concept module has **41 source files** (25,760 LOC) but its integration test directory covers only a fraction.

**Critical untested files:**
- `type_/type_manager/validation/operation_time_validation.rs` (3,875 lines) — Schema validation logic
- `type_/type_manager.rs` (3,537 lines) — Core type management
- `thing/thing_manager.rs` (3,012 lines) — Core data management

**Recommended tests:**
- Unit tests for individual validation rules
- Edge cases in type hierarchy operations (cycles, diamond inheritance)
- Error path testing for invalid schema modifications

### 6. Diagnostics Module (P2 — Medium)

**2,663 LOC** with **zero tests**. Handles metrics collection, monitoring, and reporting.

**What's untested:**
- `metrics.rs` (854 LOC) — ServerMetrics, LoadMetrics, ActionMetrics, ErrorMetrics tracking
- `reporter.rs` (363 LOC) — Metrics reporting pipeline
- `reports/json_monitoring.rs` (420 LOC) — JSON report generation
- `reports/posthog.rs` (266 LOC) — Analytics report generation
- `monitoring_server.rs` — Monitoring endpoint

**Recommended tests:**
- Metrics increment/decrement and aggregation correctness
- JSON report output format validation
- Prometheus metric export format validation
- Reporter scheduling behavior

### 7. Answer Module (P2 — Medium)

**666 LOC** with **zero tests**. Handles query result representation.

**What's untested:**
- `lib.rs` (322 LOC) — Concept type definitions (Type, Thing, Value enums), query answer structures
- `variable.rs` (168 LOC) — Query variable handling
- `variable_value.rs` (176 LOC) — Variable value representations

**Recommended tests:**
- Variable value type conversions and comparisons
- Answer construction and access patterns
- Edge cases in value representation (nulls, large values, special characters)

### 8. User Module (P2 — Medium)

**230 LOC** with **zero tests**. Manages users and permissions.

**What's untested:**
- `user_manager.rs` (132 LOC) — User CRUD operations, default user initialization
- `permission_manager.rs` (35 LOC) — Permission checks

**Recommended tests:**
- User creation/deletion/update lifecycle
- Permission grant/revoke/check
- Default admin user initialization
- Error cases (duplicate users, non-existent users)

### 9. Query Module — Beyond Define/Fetch (P2 — Medium)

The query module has **12 source files** but only **3 test files** (define, fetch, unimplemented).

**What's untested:**
- `redefine.rs` (1,338 lines) — Schema redefinition operations
- `undefine.rs` (1,228 lines) — Schema removal operations
- Query caching behavior
- ANALYSE query functionality
- Query manager lifecycle

### 10. Property-Based Testing and Fuzzing (P3 — Strategic)

The codebase has **no property-based testing** (no proptest/quickcheck) and **no fuzzing**.

**Recommended additions:**
- **Property-based tests** for the encoding layer (encode → decode round-trip invariants)
- **Property-based tests** for byte array operations (ordering properties, concatenation)
- **Fuzzing** for TypeQL query parsing — the parser is a critical security and reliability boundary
- **Property-based tests** for MVCC/snapshot isolation invariants

### 11. Performance Regression Testing (P3 — Strategic)

Only **1 benchmark file** exists (`bench_iam.rs`). Benchmarks don't appear to run in CI.

**Recommended additions:**
- Storage-layer microbenchmarks (read/write throughput, iteration speed)
- Query compilation benchmarks (time to compile various query patterns)
- Executor benchmarks for common query shapes
- Integration of benchmarks into CI with regression detection

## Priority Summary

| Priority | Area | Impact |
|---|---|---|
| **P0** | Server authentication/authorization | Security vulnerability risk |
| **P0** | Server service layers (gRPC/HTTP) | Client-facing correctness |
| **P1** | Compiler deeper coverage | Query correctness |
| **P1** | Common/bytes, structural_equality | Data integrity |
| **P1** | Common/lending_iterator adaptors | Query engine correctness |
| **P1** | Concept type validation | Schema correctness |
| **P2** | Diagnostics module | Monitoring reliability |
| **P2** | Answer module | Result correctness |
| **P2** | User module | Auth management |
| **P2** | Query module gaps | Feature coverage |
| **P3** | Property-based testing / fuzzing | Robustness |
| **P3** | Performance regression benchmarks | Long-term performance |
