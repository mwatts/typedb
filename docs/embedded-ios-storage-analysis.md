# TypeDB Embedded & iOS Feasibility Analysis

**Date:** 2026-02-20
**Codebase:** TypeDB 3.8.0 (Rust, `master` branch @ `3532e0e5b`)
**Scope:** iOS targeting, embedded SDK potential, alternative storage backends

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [iOS Compatibility Blockers](#2-ios-compatibility-blockers)
3. [Embedding Feasibility](#3-embedding-feasibility)
4. [Storage Architecture & RocksDB Usage](#4-storage-architecture--rocksdb-usage)
5. [Alternative Backend: redb](#5-alternative-backend-redb)
6. [Alternative Backend: TursoDB / libSQL](#6-alternative-backend-tursodb--libsql)
7. [Recommended Approach](#7-recommended-approach)

---

## 1. Executive Summary

TypeDB is architected as a standalone server with gRPC and HTTP APIs. Its core database engine, however, is well-separated from the networking layer and demonstrates excellent modularity. **Embedding is architecturally feasible with moderate effort.** iOS targeting introduces additional constraints, the most significant being the RocksDB dependency.

| Question | Verdict |
|----------|---------|
| Can TypeDB target iOS? | **Not currently.** Several critical blockers exist, but none are fundamental to the database engine itself. |
| Can TypeDB be embedded as a library? | **Yes, with moderate effort.** The core engine is well-separated from the server layer. |
| Can RocksDB be replaced with redb? | **Feasible with significant work.** Core KV operations map well; advanced features (prefix bloom filters, iterator pooling) require workarounds. |
| Can RocksDB be replaced with TursoDB/SQLite? | **Poor fit.** The byte-level KV access patterns are architecturally mismatched with SQL-oriented storage. |

---

## 2. iOS Compatibility Blockers

### 2.1 Critical Blockers

#### RocksDB Native Dependency
- **Location:** `storage/Cargo.toml` — `rocksdb = "0.23.0"` with `lz4` feature
- **Issue:** RocksDB is a C++ library requiring `librocksdb-sys`, `bindgen`, `libc`, `bzip2-sys`, `libz-sys`, `lz4-sys`. Cross-compiling the C++ toolchain for iOS `aarch64-apple-ios` is complex but not impossible. The real concern is RocksDB's use of `mmap` and background threads for compaction, which interact poorly with iOS app lifecycle constraints (backgrounding, memory pressure jetsam).
- **Severity:** CRITICAL — this is the primary storage engine

#### Server-First Architecture
- **Location:** `main.rs`, `server/lib.rs`
- **Issue:** TypeDB launches as a standalone process with `tokio::runtime::Builder::new_multi_thread()`, binds TCP sockets via `tonic::transport::Server` (gRPC) and `axum` (HTTP), and manages lifecycle via Unix signals (`tokio::signal::ctrl_c()`). iOS apps cannot run persistent background server processes, bind to arbitrary ports, or receive POSIX signals.
- **Severity:** CRITICAL — but solvable since the server layer is separable from the engine

#### Tokio `full` Feature Set
- **Location:** All major crates depend on `tokio = "1.47.1"` with `features = ["full"]`
- **Issue:** The `full` feature enables `signal`, `process`, `mio`, `socket2` — several of which have limited or no iOS support. However, `tokio` itself does compile for iOS when features are constrained to `rt`, `sync`, `macros`, `time`, `fs`.
- **Severity:** HIGH — requires feature-gating Tokio features per target platform

#### lz4 Native Dependency (Durability Layer)
- **Location:** `durability/Cargo.toml` — `lz4 = "1.28.1"`
- **Issue:** The `lz4` crate uses `lz4-sys` (C library). This is a separate dependency from RocksDB's LZ4 usage. Would need cross-compilation for iOS or replacement with a pure-Rust LZ4 implementation (e.g., `lz4_flex`).
- **Severity:** MODERATE — straightforward to swap

### 2.2 Secondary Issues

| Issue | Location | Notes |
|-------|----------|-------|
| `std::process::exit(1)` | `server/lib.rs:269` | iOS apps must not call `exit()` |
| `sysinfo` crate | `diagnostics/Cargo.toml` | System introspection; limited on iOS |
| `sentry` crash reporting | `Cargo.toml`, `diagnostics/` | Uses `ureq` HTTP client; may not function in iOS sandbox |
| File system assumptions | `storage/`, `durability/` | `create_dir_all`, `fsync` — work within iOS sandbox but paths must be app-container-relative |
| `chrono` platform features | Throughout | Enables `android-tzdata`, `winapi` etc. — harmless but indicates no mobile consideration |

### 2.3 What Is NOT a Blocker

- **Pure Rust query engine:** The compiler, executor, IR, concept, and encoding crates are pure Rust with no platform-specific dependencies.
- **MVCC implementation:** Entirely in-process, no OS-specific features.
- **DurabilityClient trait:** Already abstract — can be reimplemented for iOS constraints.
- **TypeQL parser:** External dependency (`typeql` git crate) — pure Rust.

---

## 3. Embedding Feasibility

### 3.1 Architectural Layers

The codebase has excellent separation of concerns across well-defined layers:

```
┌─────────────────────────────────────────────────┐
│  Binary (main.rs)                               │  CLI parsing, runtime setup
│  Server (server/)                               │  gRPC, HTTP, auth, diagnostics
├─────────────────────────────────────────────────┤
│  Database (database/)                           │  DatabaseManager, Transaction lifecycle
│  Query (query/, compiler/, executor/, ir/)      │  TypeQL compilation & execution
│  Concept (concept/)                             │  Type system, thing/attribute semantics
├─────────────────────────────────────────────────┤
│  Storage (storage/)                             │  MVCCStorage<D>, snapshots, isolation
│  Encoding (encoding/)                           │  Binary key/value encoding
│  Durability (durability/)                       │  WAL, crash recovery
├─────────────────────────────────────────────────┤
│  Common (common/*)                              │  bytes, error, logger, concurrency, etc.
│  Resource (resource/)                           │  Constants, configuration
└─────────────────────────────────────────────────┘
```

**Key insight:** The `database` crate is the integration hub that depends on all core components. The `server` crate is a thin wrapper that adds networking and authentication on top. This boundary is the natural SDK extraction point.

### 3.2 Transaction API (Already Embeddable)

The transaction API is protocol-agnostic and fully usable without networking:

```rust
// database/transaction.rs
pub struct TransactionRead<D: DurabilityClient> {
    pub snapshot: Arc<ReadSnapshot<D>>,
    pub type_manager: Arc<TypeManager>,
    pub thing_manager: Arc<ThingManager>,
    pub function_manager: Arc<FunctionManager>,
    pub query_manager: Arc<QueryManager>,
    pub database: DatabaseDropGuard<D>,
    transaction_options: TransactionOptions,
    pub profile: TransactionProfile,
}

impl<D: DurabilityClient> TransactionRead<D> {
    pub fn open(database: Arc<Database<D>>, options: TransactionOptions) -> Result<Self, ...>
}
```

Similarly, `TransactionWrite<D>` and `TransactionSchema<D>` are generic over the durability client.

### 3.3 Database Manager (Clean Entry Point)

```rust
// database/database_manager.rs
pub struct DatabaseManager {
    databases: RwLock<HashMap<String, Arc<Database<WALClient>>>>,
}

impl DatabaseManager {
    pub fn new(data_directory: impl AsRef<Path>) -> Result<Arc<Self>, ...>
    pub fn put_database(&self, name: impl AsRef<str>) -> Result<(), ...>
    pub fn get_database(&self, name: &str) -> Option<Arc<Database<WALClient>>>
}
```

### 3.4 Global State Assessment

- **No `static mut` in core crates** — safe for multi-instance
- **No `lazy_static!` / `once_cell` in database or storage** — no hidden singletons
- **No `thread_local!` in core** — safe for multi-threaded embedding
- **Logging:** Uses `tracing` subscriber (initialized in `main.rs`). An embedding host can provide its own subscriber.

### 3.5 What Needs Work for an SDK Crate

| Component | Current State | Required Change | Effort |
|-----------|---------------|-----------------|--------|
| Network layer (gRPC/HTTP) | Tightly bound to `server` | Exclude from SDK crate | Low — just don't include `server/` |
| Authentication | Embedded in `ServerState` trait | Bypass or implement dummy auth for embedded mode | Low |
| Configuration | File-based YAML + CLI | Add programmatic `ConfigBuilder::new()` | Low |
| User management | System database with user tables | Optional; single-user embedded mode | Low |
| Diagnostics | Integrated into all operations | Feature-gate as optional | Moderate |
| Tokio runtime | Requires `full` features | Reduce to `rt`, `sync`, `time`, `macros` | Moderate |

### 3.6 Proposed SDK Crate Structure

```rust
// typedb-embedded/src/lib.rs
pub struct TypeDB {
    database_manager: Arc<DatabaseManager>,
}

impl TypeDB {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, Error> { ... }
    pub fn create_database(&self, name: &str) -> Result<(), Error> { ... }
    pub fn database(&self, name: &str) -> Option<Database> { ... }
    pub fn databases(&self) -> Vec<String> { ... }
}

pub struct Database { /* wraps Arc<database::Database<WALClient>> */ }

impl Database {
    pub fn transaction_read(&self, options: TransactionOptions) -> Result<ReadTxn, Error> { ... }
    pub fn transaction_write(&self, options: TransactionOptions) -> Result<WriteTxn, Error> { ... }
    pub fn transaction_schema(&self, options: TransactionOptions) -> Result<SchemaTxn, Error> { ... }
}
```

**Estimated effort:** 2-4 weeks for experienced Rust developers, primarily wiring up existing components with a clean public API.

---

## 4. Storage Architecture & RocksDB Usage

### 4.1 MVCC Key Format

TypeDB implements its own MVCC layer on top of RocksDB's raw key-value store. Keys are stored as:

```
[USER_KEY][SEQUENCE_NUMBER_INVERTED (8 bytes)][OPERATION (1 byte)]
```

- **USER_KEY:** Variable-length application data (schema, entity IDs, attributes, links, etc.)
- **SEQUENCE_NUMBER_INVERTED:** 8 bytes, bitwise-inverted so that the most recent version sorts first in byte order (enabling efficient prefix seeks for "latest version")
- **OPERATION:** `0x0` = Insert, `0x1` = Delete

This means multiple versions of the same logical key coexist in storage. Reads at a given sequence number scan from the prefix and find the first version <= their snapshot.

### 4.2 Keyspace Organization

TypeDB uses **5 separate RocksDB database instances** (not column families), each optimized with different prefix lengths for bloom filter efficiency:

| Keyspace | Prefix Length | Contents |
|----------|--------------|----------|
| `DefaultOptimisedPrefix11` | 11 bytes | Schema, short attribute vertices, object existence checks |
| `OptimisedPrefix15` | 15 bytes | Links, links reverse, has-edges |
| `OptimisedPrefix16` | 16 bytes | Has-reverse for short attributes |
| `OptimisedPrefix17` | 17 bytes | Long attribute existence, links index |
| `OptimisedPrefix25` | 25 bytes | Has-reverse for long attributes |

Each keyspace has its own RocksDB `SliceTransform::create_fixed_prefix(N)` and bloom filter configuration. This is a critical performance optimization — it means point lookups and prefix scans within a keyspace hit bloom filters tuned to that keyspace's key structure.

**Source:** `encoding/encoding.rs` — `EncodingKeyspace` enum and `KeyspaceSet` trait implementation.

### 4.3 RocksDB Features Used

| Feature | Used? | Details |
|---------|-------|---------|
| Prefix extractors | **Yes, heavily** | Per-keyspace fixed-prefix extractors (11, 15, 16, 17, 25 bytes) |
| Bloom filters | **Yes, heavily** | 10-bit bloom, partitioned, L0 pinned in cache |
| LRU block cache | **Yes** | Shared cache across keyspaces, configurable size |
| LZ4 compression | **Yes** | Levels 2-6; levels 0-1 uncompressed for write performance |
| Raw iterators | **Yes** | `DBRawIterator` with prefix seek and total-order seek modes |
| Iterator pooling | **Yes** | Custom `IteratorPool` reuses RocksDB iterator instances |
| Write batches | **Yes** | `WriteBatch` for atomic multi-key commits |
| Checkpoints | **Yes** | `rocksdb::checkpoint::Checkpoint` for consistent snapshots/exports |
| WAL | **Disabled** | `write_options.disable_wal(true)` — TypeDB uses its own WAL |
| Transactions API | **Not used** | TypeDB implements its own MVCC with `IsolationManager` |
| Column families | **Not used** | Separate DB instances instead |
| Merge operators | **Not used** | |
| Compaction filters | **Not used** | |
| Custom comparators | **Not used** | Standard byte ordering |

### 4.4 Existing Abstractions

The codebase has partial abstraction that aids backend replacement:

- **`DurabilityClient` trait** (`storage/durability_client.rs`): Fully abstract. The WAL is decoupled from RocksDB. Any backend that provides sequenced writes and replay iteration can implement this.
- **`KeyspaceSet` trait** (`storage/keyspace/keyspace.rs`): Defines keyspace enumeration but includes `fn rocks_configuration(&self, cache: &rocksdb::Cache) -> rocksdb::Options` — tightly coupled to RocksDB.
- **`ReadableSnapshot` / `WritableSnapshot` traits** (`storage/snapshot/snapshot.rs`): Abstract over read/write operations using `StorageKey` / `ByteArray` types — but the underlying storage calls go directly to RocksDB.
- **No generic `StorageBackend` trait** exists. RocksDB `DB` instances are used directly in `Keyspace`.

### 4.5 Write Path

```
Transaction.commit()
  → IsolationManager.validate_commit()     // checks for conflicts
    → DurabilityClient.sequenced_write()   // writes commit record to WAL
    → WriteBatches::new()                  // creates rocksdb::WriteBatch per keyspace
      → db.write_opt(batch, &write_options) // applies to RocksDB
    → IsolationManager.applied()           // marks as visible to new readers
```

### 4.6 Read Path

```
Snapshot.get(key)
  → Check in-memory OperationsBuffer (local writes)
  → If not found: rocksdb::DB::get_pinned_opt(mvcc_key)
  → Decode MVCC version, find latest version <= snapshot sequence number

Snapshot.iterate_range(range)
  → Create merged iterator over:
    1. In-memory buffer (BTreeMap)
    2. RocksDB raw iterator (prefix seek or total-order seek)
  → Merge by key order, apply MVCC visibility filtering
```

---

## 5. Alternative Backend: redb

### 5.1 Overview

[redb](https://github.com/cberner/redb) is a pure-Rust embedded key-value store with ACID transactions, B+ tree storage, and zero-copy reads. It compiles to all Rust targets including iOS.

### 5.2 Compatibility Assessment

| Requirement | redb Support | Gap |
|-------------|-------------|-----|
| Ordered key-value storage | Yes — B+ tree with byte-ordered keys | None |
| Range scans | Yes — `range()` API on tables | None |
| Prefix scans | Partial — range scans with prefix bounds work, but no bloom filter acceleration | **Performance gap** |
| Atomic write batches | Yes — write transactions are atomic | None |
| Multiple keyspaces | Yes — multiple named tables | None |
| Concurrent readers | Yes — MVCC with multiple read transactions | None |
| Snapshots / checkpoints | No built-in checkpoint API | **Must implement manually** (file copy while DB is locked, or use savepoints) |
| Bloom filters | No | **Performance gap** for point lookups |
| Prefix extractors | No | **Must implement custom prefix indexing or accept slower prefix scans** |
| Iterator pooling | No — iterators are tied to transaction lifetime | **Lifetime model change** |
| Compression | No built-in compression | **Storage size increase** (~2-3x for compressible data) |
| Block cache control | No — relies on OS page cache | **Less predictable memory** |
| Raw byte iteration | Yes — `range()` returns `(&[u8], &[u8])` | None |
| Write performance (LSM vs B+tree) | Different tradeoffs — B+ tree has lower write amplification but higher write latency for large batches | **Benchmarking needed** |

### 5.3 Implementation Path

1. **Create a `StorageBackend` trait** abstracting over RocksDB and redb:
   ```rust
   pub trait StorageBackend: Send + Sync + 'static {
       type ReadTxn<'a>: StorageReadTransaction + 'a where Self: 'a;
       type WriteTxn<'a>: StorageWriteTransaction + 'a where Self: 'a;
       fn read_txn(&self) -> Result<Self::ReadTxn<'_>, StorageError>;
       fn write_txn(&self) -> Result<Self::WriteTxn<'_>, StorageError>;
   }
   ```

2. **Adapt `Keyspace`** to be generic over the backend rather than hardcoded to `rocksdb::DB`.

3. **Map keyspaces to redb tables** — one `redb::TableDefinition` per `EncodingKeyspace`.

4. **Accept performance regression** on prefix scans — redb range scans without bloom filters will be slower for point lookups in large datasets. Mitigate with application-level bloom filter (e.g., `probabilistic-collections` crate).

5. **Implement checkpoint** via redb's `savepoint()` API or filesystem-level copy.

### 5.4 Verdict

**Feasible with significant effort.** The core KV operations (get, put, delete, range scan, atomic batches) map cleanly. The main gaps are performance optimizations (bloom filters, prefix extractors, compression, iterator pooling) that would need application-level replacements or acceptance of degraded performance. The pure-Rust nature of redb makes it ideal for iOS and embedded targets.

**Estimated effort:** 4-6 weeks. The `Keyspace` and `MVCCStorage` layers need refactoring to be generic over the backend, and the snapshot/iterator code needs adaptation for redb's transaction-scoped iterator lifetime model.

---

## 6. Alternative Backend: TursoDB / libSQL

### 6.1 Overview

[TursoDB](https://turso.tech/) is a fork of SQLite (libSQL) with embedded replicas, vector search, and async I/O. It provides a SQL interface over a relational/document storage engine.

### 6.2 Compatibility Assessment

| Requirement | TursoDB/SQLite Support | Gap |
|-------------|----------------------|-----|
| Ordered key-value storage | Via SQL table with `BLOB PRIMARY KEY` | **Impedance mismatch** — SQL overhead for raw KV |
| Range scans | Via `WHERE key >= ? AND key < ?` | **Overhead** — SQL parsing per scan |
| Prefix scans | Via `WHERE key >= ? AND key < ?` with prefix bounds | **No bloom filter equivalent** |
| Atomic write batches | Via SQL transactions | Compatible but **higher overhead** |
| Multiple keyspaces | Via separate tables | Compatible |
| Concurrent readers | SQLite WAL mode allows concurrent reads | Compatible |
| Snapshots / checkpoints | `VACUUM INTO` or file copy | Compatible |
| Compression | Limited (ZSTD in newer SQLite) | Partial |
| MVCC key format | Could store in BLOB columns | **Requires encoding/decoding at SQL boundary** |
| Raw byte iteration | Via cursors, but with SQL overhead | **Significant overhead** |
| Iterator pooling | Prepared statements could cache | **Different model** |
| Write throughput | SQLite single-writer limitation | **Bottleneck for concurrent writes** |

### 6.3 Fundamental Mismatches

1. **SQL overhead for byte-level operations:** TypeDB's storage layer operates on raw bytes with microsecond-level operations. Every access through SQLite adds SQL parsing, query planning, and result marshaling overhead — even for simple `SELECT value FROM kv WHERE key = ?`.

2. **Single-writer constraint:** SQLite allows only one writer at a time (WAL mode helps readers but not writers). TypeDB's current architecture with RocksDB write batches is inherently more concurrent.

3. **No prefix-optimized iteration:** SQLite's B-tree index on `BLOB` keys will handle range scans, but without the bloom filter acceleration that TypeDB relies on for prefix-based access patterns.

4. **Double MVCC:** TypeDB implements its own MVCC with sequence numbers baked into keys. SQLite also implements MVCC internally. Running two MVCC layers is wasteful and adds complexity. A TursoDB backend would either need to strip TypeDB's MVCC (massive rewrite) or accept the redundancy.

### 6.4 Verdict

**Poor fit.** The architectural mismatch between TypeDB's raw byte-level KV access patterns and SQLite's SQL-oriented interface makes this impractical. While technically possible (any KV store can be emulated over SQL), the performance overhead would be severe (estimated 5-20x slowdown for typical operations), and the implementation would fight against SQLite's strengths rather than leveraging them.

**Not recommended** unless the goal is specifically to use TursoDB's replication/sync features, in which case the key-value operations could be wrapped in a thin SQL layer with prepared statements — but expect significant performance degradation.

---

## 7. Recommended Approach

### Phase 1: Create `typedb-embedded` SDK Crate (2-4 weeks)

Extract the database engine from the server shell:

- Create a new crate wrapping `DatabaseManager`, `Database<WALClient>`, and transaction types
- Provide a programmatic configuration API (no YAML files required)
- Feature-gate diagnostics, user management, and authentication as optional
- Reduce Tokio features to the minimal set (`rt`, `sync`, `time`, `macros`)
- Expose a clean public API for database lifecycle, transactions, and TypeQL query execution

### Phase 2: Abstract the Storage Backend (4-6 weeks)

Introduce a `StorageBackend` trait to decouple from RocksDB:

- Create trait abstracting `Keyspace` operations (get, put, delete, range scan, write batch, checkpoint)
- Implement the trait for RocksDB (preserving current behavior)
- Implement the trait for redb (new backend)
- Make `MVCCStorage<D>` also generic over the storage backend: `MVCCStorage<D, S: StorageBackend>`
- Adapt `KeyspaceSet` trait to remove `rocks_configuration()` and replace with backend-agnostic configuration

### Phase 3: iOS Target Support (2-3 weeks, after Phase 2 with redb)

With redb as the storage backend:

- Replace `lz4` crate with `lz4_flex` (pure Rust) in the durability layer
- Constrain Tokio features for iOS targets via `#[cfg(target_os = "ios")]`
- Remove signal handling; provide API for host app to trigger graceful shutdown
- Test cross-compilation to `aarch64-apple-ios` and `aarch64-apple-ios-sim`
- Create Swift/C bindings via `uniffi` or `cbindgen`

### Priority Order

If the goal is **iOS embedded database**, the recommended sequence is:

1. Phase 1 (SDK crate) — validates embeddability, provides immediate value
2. Phase 2 (redb backend) — removes C++ dependency chain, enables iOS
3. Phase 3 (iOS targeting) — final platform-specific adaptations

If the goal is **embedded on desktop/server only** (no iOS), Phase 1 alone may suffice, keeping RocksDB as the storage backend.

---

## Appendix: Key Source Files

| File | Relevance |
|------|-----------|
| `storage/storage.rs` | `MVCCStorage<D>`, MVCC key encoding, read/write paths |
| `storage/keyspace/keyspace.rs` | `Keyspace`, `Keyspaces`, `KeyspaceSet` trait, RocksDB initialization |
| `storage/keyspace/raw_iterator.rs` | Iterator pooling, raw byte iteration |
| `storage/isolation_manager.rs` | `IsolationManager`, commit validation, timeline management |
| `storage/durability_client.rs` | `DurabilityClient` trait (abstract WAL interface) |
| `storage/write_batches.rs` | `WriteBatches` wrapping `rocksdb::WriteBatch` |
| `storage/snapshot/snapshot.rs` | `ReadableSnapshot`, `WritableSnapshot` traits |
| `encoding/encoding.rs` | `EncodingKeyspace`, RocksDB per-keyspace configuration |
| `database/database_manager.rs` | `DatabaseManager` — top-level database lifecycle |
| `database/transaction.rs` | `TransactionRead<D>`, `TransactionWrite<D>` |
| `server/lib.rs` | Server startup, signal handling, gRPC/HTTP binding |
| `main.rs` | Binary entry point, CLI, runtime setup |
| `durability/wal.rs` | Write-ahead log implementation |
