# TypeDB Embedded Rust Driver Design

**Date:** 2026-02-20
**Crate Name:** `typedb-embedded`
**Scope:** Drop-in replacement for `typedb-driver` (Rust) that embeds the full database engine, eliminating gRPC/network overhead.

---

## Table of Contents

1. [Goals & Non-Goals](#1-goals--non-goals)
2. [Driver API Surface](#2-driver-api-surface)
3. [Architecture](#3-architecture)
4. [Server-Side Implementation Blueprint](#4-server-side-implementation-blueprint)
5. [API Mapping: Remote Driver to Embedded](#5-api-mapping-remote-driver-to-embedded)
6. [Crate Structure](#6-crate-structure)
7. [Type Mapping: Engine Internals to Driver API](#7-type-mapping-engine-internals-to-driver-api)
8. [Transaction Lifecycle](#8-transaction-lifecycle)
9. [Query Execution Path](#9-query-execution-path)
10. [Error Handling](#10-error-handling)
11. [Async/Sync Duality](#11-asyncsync-duality)
12. [Feature Flags](#12-feature-flags)
13. [Dependencies](#13-dependencies)
14. [Migration Path for Existing Users](#14-migration-path-for-existing-users)
15. [Open Questions](#15-open-questions)

---

## 1. Goals & Non-Goals

### Goals

- Expose the **same public API** as `typedb-driver` (Rust) so users can switch by changing `Cargo.toml` and one import line
- Embed the full TypeDB engine: storage, MVCC, query compiler, executor, type system
- Zero network overhead — all operations are direct function calls into the engine
- Support both sync and async usage via a `sync` feature flag (matching the remote driver)
- Single-process, multi-database operation
- Thread-safe: multiple threads can hold concurrent read transactions

### Non-Goals

- gRPC/HTTP server capabilities (use TypeDB server for that)
- Replication, clustering, or multi-node features (`replicas_info()` etc. return stubs)
- User management / authentication (embedded mode is single-user; `UserManager` returns stubs)
- Export/import via streaming protocol (provide direct file-based equivalents instead)
- iOS/mobile support in the initial version (see `embedded-ios-storage-analysis.md` for Phase 2/3)

---

## 2. Driver API Surface

The remote `typedb-driver` exposes these public types:

### Entry Point
```rust
pub struct TypeDBDriver {
    // Remote: holds ServerConnection, BackgroundRuntime
    // Embedded: holds DatabaseManager, data directory
}

impl TypeDBDriver {
    pub async fn new(address, credentials, driver_options) -> Result<Self>;
    pub fn is_open(&self) -> bool;
    pub fn databases(&self) -> &DatabaseManager;
    pub fn users(&self) -> &UserManager;
    pub async fn transaction(database_name, transaction_type) -> Result<Transaction>;
    pub async fn transaction_with_options(database_name, transaction_type, options) -> Result<Transaction>;
    pub fn force_close(&self) -> Result;
}
```

### Database Management
```rust
pub struct DatabaseManager { .. }
impl DatabaseManager {
    pub async fn all(&self) -> Result<Vec<Arc<Database>>>;
    pub async fn get(&self, name) -> Result<Arc<Database>>;
    pub async fn contains(&self, name) -> Result<bool>;
    pub async fn create(&self, name) -> Result;
    pub async fn import_from_file(&self, name, schema, data_file_path) -> Result;
}

pub struct Database { .. }
impl Database {
    pub fn name(&self) -> &str;
    pub async fn delete(self: Arc<Self>) -> Result;
    pub async fn schema(&self) -> Result<String>;
    pub async fn type_schema(&self) -> Result<String>;
    pub async fn export_to_file(&self, schema_path, data_path) -> Result;
    pub fn replicas_info(&self) -> Vec<ReplicaInfo>;           // stub in embedded
    pub fn primary_replica_info(&self) -> Option<ReplicaInfo>;  // stub in embedded
    pub fn preferred_replica_info(&self) -> Option<ReplicaInfo>; // stub in embedded
}
```

### Transactions
```rust
pub struct Transaction { .. }
impl Transaction {
    pub fn is_open(&self) -> bool;
    pub fn type_(&self) -> TransactionType;
    pub fn query(&self, query) -> impl Promise<'static, Result<QueryAnswer>>;
    pub fn query_with_options(&self, query, options) -> impl Promise<'static, Result<QueryAnswer>>;
    pub fn analyze(&self, query) -> impl Promise<'static, Result<AnalyzedQuery>>;
    pub fn on_close(&self, callback) -> impl Promise<'_, Result<()>>;
    pub fn close(&self) -> impl Promise<'_, Result<()>>;
    pub fn commit(self) -> impl Promise<'static, Result>;
    pub fn rollback(&self) -> impl Promise<'_, Result>;
}
```

### Answer Types
```rust
pub enum QueryAnswer {
    Ok(QueryType),
    ConceptRowStream(Arc<ConceptRowHeader>, BoxStream<'static, Result<ConceptRow>>),
    ConceptDocumentStream(Arc<ConceptDocumentHeader>, BoxStream<'static, Result<ConceptDocument>>),
}

pub enum QueryType { ReadQuery, WriteQuery, SchemaQuery }
pub enum TransactionType { Read = 0, Write = 1, Schema = 2 }
```

### Concept Types
```rust
pub enum Concept {
    EntityType(EntityType), RelationType(RelationType),
    RoleType(RoleType), AttributeType(AttributeType),
    Entity(Entity), Relation(Relation), Attribute(Attribute),
    Value(Value),
}
// + all accessor methods (try_get_iid, get_label, try_get_value, etc.)
```

### Common Types
```rust
pub struct Credentials { .. }
pub struct DriverOptions { .. }
pub struct TransactionOptions { .. }
pub struct QueryOptions { .. }
pub type IID = id::ID;
pub type Result<T = ()> = std::result::Result<T, Error>;
pub struct Error { .. }
```

---

## 3. Architecture

### Remote Driver (current)
```
User Code
    ↓
typedb-driver (Rust API)
    ↓
gRPC (tonic) ──network──→ TypeDB Server
                                ↓
                          database/ query/ storage/ ...
                                ↓
                            RocksDB
```

### Embedded Driver (proposed)
```
User Code
    ↓
typedb-embedded (same Rust API)
    ↓  (direct function calls, no serialization)
database/ query/ storage/ ...
    ↓
RocksDB
```

The key insight is that the remote driver's API is a thin wrapper around gRPC calls, while the engine's internal `DatabaseManager`, `TransactionRead/Write/Schema`, and `QueryManager` already provide the same functionality as direct function calls. `typedb-embedded` bridges these two, translating the driver API into engine calls.

### Dependency Graph
```
typedb-embedded
    ├── database     (DatabaseManager, Database<WALClient>, Transaction*)
    ├── query        (QueryManager)
    ├── compiler     (query compilation)
    ├── executor     (query execution)
    ├── ir           (intermediate representation)
    ├── concept      (type system)
    ├── function     (built-in + user-defined functions)
    ├── storage      (MVCCStorage, snapshots)
    ├── durability   (WAL)
    ├── encoding     (key encoding, keyspaces)
    ├── resource     (constants, config)
    ├── answer       (internal result types)
    ├── common/*     (error, logger, bytes, concurrency, etc.)
    └── typeql       (parser)
```

**Not included:** `server/`, `user/`, `system/`, `diagnostics/` (optional via feature flag).

---

## 4. Server-Side Implementation Blueprint

The server-side gRPC/HTTP handlers are the authoritative reference for how driver API calls translate into engine operations. The embedded driver should replicate these exact code paths, minus the protobuf serialization and network transport. The key source files are:

### Key Server Files

| File | What It Shows |
|------|---------------|
| `server/service/transaction_service.rs` | The `Transaction` enum wrapping `TransactionRead<WALClient>` / `TransactionWrite<WALClient>` / `TransactionSchema<WALClient>`. The `with_readable_transaction!` macro for dispatching across transaction types. Error types for the transaction service. |
| `server/service/grpc/transaction_service.rs` | **The most critical file.** Complete implementation of: transaction open, query dispatch (read/write/schema), commit, rollback, close, query streaming, and interrupt handling. This is the exact code the embedded driver must replicate. |
| `server/service/grpc/typedb_service.rs` | Database management: `databases_all`, `databases_get`, `databases_contains`, `databases_create`, `database_delete`, `database_schema`, `database_type_schema`, `database_export`, `databases_import`. Each delegates to `ServerState` methods. |
| `server/state.rs` | `ServerState` trait and `LocalServerState` implementation. Shows the exact engine calls behind each API: `database_manager.database_names()`, `database_manager.put_database()`, `database_manager.delete_database()`, plus `get_database_schema()` / `get_database_type_schema()` which open read transactions internally. |
| `server/service/grpc/row.rs` | `encode_row()` — converts engine `MaybeOwnedRow` + `VariablePosition` columns into protocol `ConceptRow`. This is the row serialization the embedded driver can skip. |
| `server/service/grpc/concept.rs` | `encode_thing_concept()`, `encode_type_concept()`, `encode_value()` — converts engine `answer::Thing`/`answer::Type`/`encoding::Value` into protocol types. The embedded driver needs equivalent conversions to its own `Concept` types instead. |
| `server/service/grpc/document.rs` | `encode_document()` — converts engine `ConceptDocument` to protocol. |
| `server/service/grpc/options.rs` | `transaction_options_from_proto()`, `query_options_from_proto()` — shows which options are forwarded from driver to engine. |

### Transaction Open (from `grpc/transaction_service.rs:500-562`)

The server opens transactions via `spawn_blocking` since engine operations are synchronous:

```rust
// Exact server code pattern:
let database = self.database_manager.database(database_name).ok_or(NotFound)?;
let transaction = match transaction_type {
    Type::Read   => Transaction::Read(spawn_blocking(|| TransactionRead::open(database, options)).await?),
    Type::Write  => Transaction::Write(spawn_blocking(|| TransactionWrite::open(database, options)).await?),
    Type::Schema => Transaction::Schema(spawn_blocking(|| TransactionSchema::open(database, options)).await?),
};
```

The embedded driver should use the same `spawn_blocking` pattern in async mode, or direct calls in sync mode.

### Query Dispatch (from `grpc/transaction_service.rs:874-927`)

The server parses TypeQL then routes by query structure:

```rust
let parsed = parse_query(&query)?;
match parsed.into_structure() {
    QueryStructure::Schema(schema_query) => {
        // Schema queries: immediate, blocks all other queries
        execute_schema_query(schema_transaction, query, source_query)
    }
    QueryStructure::Pipeline(pipeline) => {
        if is_write_pipeline(&pipeline) {
            // Write queries: queued, executed serially via spawn_blocking
            execute_write_query_in_write(write_txn, options, pipeline, source, interrupt)
            // or execute_write_query_in_schema(schema_txn, ...)
        } else {
            // Read queries: can run concurrently, use spawn_blocking
            query_manager.prepare_read_pipeline(snapshot, type_mgr, thing_mgr, func_mgr, &pipeline, &source)
            // then iterate with pipeline.into_rows_iterator() or pipeline.into_documents_iterator()
        }
    }
}
```

### Read Query Execution (from `grpc/transaction_service.rs:1264-1466`)

The `blocking_read_query_worker` method shows the full read path:

1. Clone `snapshot`, `type_manager`, `thing_manager`, `function_manager`, `query_manager` from the transaction (all `Arc`-wrapped, cheap to clone)
2. Call `query_manager.prepare_read_pipeline(snapshot, &type_manager, thing_manager, &function_manager, &pipeline, &source_query)`
3. Check `pipeline.has_fetch()` — if true, use `pipeline.into_documents_iterator(interrupt)`, otherwise use `pipeline.into_rows_iterator(interrupt)`
4. For rows: get `pipeline.rows_positions()` for column names, then iterate calling `encode_row()` on each `MaybeOwnedRow`
5. For documents: iterate calling `encode_document()` on each `ConceptDocument`

The embedded driver replaces step 4/5's protobuf encoding with conversion to driver `ConceptRow`/`ConceptDocument` types.

### Write Query Execution (from `grpc/transaction_service.rs:1056-1082`)

Write queries take ownership of the transaction, execute, and return it:

```rust
// Schema transaction + write pipeline:
let (transaction, result) = execute_write_query_in_schema(schema_txn, options, pipeline, source, interrupt);
// Write transaction + write pipeline:
let (transaction, result) = execute_write_query_in_write(write_txn, options, pipeline, source, interrupt);
```

Both functions are in `database/query.rs`. They destructure the transaction, run the query, and reassemble it. The result is `WriteQueryAnswer` containing either a `Batch` (rows) or `Vec<ConceptDocument>` (documents).

### Commit (from `grpc/transaction_service.rs:564-628`)

```rust
match transaction {
    Transaction::Read(_) => error("cannot commit read"),
    Transaction::Write(txn) => { let (profile, result) = txn.commit(); result? }
    Transaction::Schema(txn) => { let (profile, result) = txn.commit(); result? }
}
```

### Schema Export (from `server/state.rs:265-281`)

`Database::schema()` and `Database::type_schema()` are implemented by opening an internal read transaction:

```rust
fn get_database_schema<D: DurabilityClient>(database: Arc<Database<D>>) -> Result<String, Error> {
    let transaction = TransactionRead::open(database, TransactionOptions::default())?;
    let schema = get_transaction_schema(&transaction)?;
    Ok(schema)
}
```

The `get_transaction_schema()` / `get_transaction_type_schema()` functions (in `server/state.rs`) format the current schema as TypeQL define statements.

### Concept Encoding (from `grpc/concept.rs`)

The server encodes engine types to protobuf. The embedded driver must do an analogous conversion to driver types:

| Server Encoding | Embedded Equivalent |
|----------------|---------------------|
| `encode_entity(entity, snapshot, type_manager)` → `typedb_protocol::Entity { iid, entity_type }` | Convert to `driver::Entity { iid: IID::from(entity.iid()), type_: entity_type_label }` |
| `encode_relation(relation, snapshot, type_manager)` → `typedb_protocol::Relation { iid, relation_type }` | Convert to `driver::Relation { iid, type_ }` |
| `encode_attribute(attribute, snapshot, type_manager, thing_manager)` → `typedb_protocol::Attribute { iid, value, attribute_type }` | Convert to `driver::Attribute { iid, value, type_ }` — note: `attribute.get_value(snapshot, thing_manager, storage_counters)` fetches the value |
| `encode_entity_type(et, snapshot, type_manager)` → `typedb_protocol::EntityType { label }` | `et.get_label(snapshot, type_manager)?.scoped_name().as_str().to_string()` |
| `encode_value(value)` → `typedb_protocol::Value` | Direct mapping of `encoding::value::Value` variants to driver `Value` variants |

### Row Result Conversion (from `grpc/row.rs`)

The `encode_row()` function shows exactly how engine rows become driver rows:

```rust
fn encode_row(row: MaybeOwnedRow, columns: &[(String, VariablePosition)], snapshot, type_manager, thing_manager, ...) {
    for (column_name, position) in columns {
        let variable_value: &VariableValue = row.get(*position);
        match variable_value {
            VariableValue::None    => empty entry,
            VariableValue::Type(t) => encode_type_concept(t, snapshot, type_manager),
            VariableValue::Thing(t) => encode_thing_concept(t, snapshot, type_manager, thing_manager, ...),
            VariableValue::Value(v) => encode_value(v),
            VariableValue::ThingList(list) => encode each thing,
            VariableValue::ValueList(list) => encode each value,
        }
    }
}
```

The embedded driver does the same, converting `VariableValue` to driver `Concept` types instead of protobuf types.

### ExecutionInterrupt Pattern

The server creates an interrupt channel per transaction:

```rust
let (query_interrupt_sender, query_interrupt_receiver) = broadcast::channel(1);
let interrupt = ExecutionInterrupt::new(query_interrupt_receiver);
```

On commit/rollback/close, it sends `InterruptType::TransactionCommitted` / `TransactionRolledback` / `TransactionClosed`. The embedded driver should replicate this pattern.

---

## 5. API Mapping: Remote Driver to Embedded

### TypeDBDriver

| Remote Driver | Embedded Implementation |
|---------------|------------------------|
| `TypeDBDriver::new(address, credentials, options)` | `TypeDBDriver::new(data_dir, options)` — constructor changes signature. `address`/`credentials` not needed. Calls `database::DatabaseManager::new(data_dir)` |
| `TypeDBDriver::is_open()` | Returns `!self.closed` flag |
| `TypeDBDriver::databases()` | Returns `&self.database_manager` (wrapper around engine's `DatabaseManager`) |
| `TypeDBDriver::users()` | Returns stub `UserManager` that reports single "embedded" user |
| `TypeDBDriver::transaction(db, type)` | Opens engine transaction directly: `TransactionRead::open()` / `TransactionWrite::open()` / `TransactionSchema::open()` |
| `TypeDBDriver::force_close()` | Drops internal `DatabaseManager`, closes all RocksDB instances |

**Constructor change:** The embedded driver cannot use the same `new(address, credentials, options)` signature since there is no server. Two approaches:

**Option A — Different constructor, same type name:**
```rust
impl TypeDBDriver {
    /// Embedded-only constructor. Opens/creates the data directory.
    pub fn new_embedded(
        data_dir: impl AsRef<Path>,
        options: EmbeddedOptions,
    ) -> Result<Self>;
}
```

**Option B — Feature-gated constructor (recommended):**
```rust
impl TypeDBDriver {
    /// Create an embedded TypeDB instance.
    /// The `address` parameter is interpreted as the data directory path.
    /// Credentials are ignored in embedded mode.
    pub async fn new(
        data_dir: impl AsRef<str>,
        _credentials: Credentials,
        _driver_options: DriverOptions,
    ) -> Result<Self>;
}
```

Option B maximizes API compatibility — existing code just changes the first argument from `"localhost:1729"` to `"./data"`.

### DatabaseManager

| Remote Driver | Embedded Implementation |
|---------------|------------------------|
| `all()` | `engine_dm.database_names()` → wrap each in driver `Database` |
| `get(name)` | `engine_dm.database(name)` → wrap in driver `Database` |
| `contains(name)` | `engine_dm.database(name).is_some()` |
| `create(name)` | `engine_dm.put_database(name)` |
| `import_from_file(name, schema, data)` | Use engine import machinery (`prepare_imported_database` + `finalise_imported_database`) |

### Database

| Remote Driver | Embedded Implementation |
|---------------|------------------------|
| `name()` | Delegate to `engine::Database::name()` |
| `delete()` | `engine_dm.delete_database(name)` |
| `schema()` | Open read transaction, execute `define` export query via `QueryManager` |
| `type_schema()` | Same as `schema()` but filtered to type definitions |
| `export_to_file()` | Open read transaction, iterate all data, serialize to file |
| `replicas_info()` | Return `vec![]` |
| `primary_replica_info()` | Return `None` |
| `preferred_replica_info()` | Return `None` |

### Transaction

| Remote Driver | Embedded Implementation |
|---------------|------------------------|
| `new(stream)` | Wraps engine `TransactionRead<WALClient>` / `TransactionWrite<WALClient>` / `TransactionSchema<WALClient>` |
| `is_open()` | Check if inner transaction hasn't been dropped/committed |
| `type_()` | Return stored `TransactionType` |
| `query(q)` | Parse TypeQL → dispatch to engine `QueryManager` → convert results |
| `query_with_options(q, opts)` | Same with options forwarded |
| `analyze(q)` | Parse TypeQL → use compiler to analyze → return `AnalyzedQuery` |
| `commit()` | Call engine `TransactionWrite::commit()` or `TransactionSchema::commit()` |
| `rollback()` | Call engine `TransactionWrite::rollback()` |
| `close()` | Drop the inner engine transaction |
| `on_close(cb)` | Store callback, invoke on drop |

---

## 6. Crate Structure

```
typedb-embedded/
├── Cargo.toml
├── src/
│   ├── lib.rs              # pub re-exports matching typedb-driver
│   ├── driver.rs           # TypeDBDriver (embedded implementation)
│   ├── database/
│   │   ├── mod.rs
│   │   ├── database.rs     # Database wrapper
│   │   ├── database_manager.rs  # DatabaseManager wrapper
│   │   └── migration.rs    # Export/import helpers
│   ├── transaction.rs      # Transaction wrapper
│   ├── answer/
│   │   ├── mod.rs          # QueryAnswer, QueryType
│   │   ├── concept_row.rs  # ConceptRow (converted from engine Batch)
│   │   └── concept_document.rs  # ConceptDocument passthrough
│   ├── concept/
│   │   ├── mod.rs          # Concept enum (mirrors driver's)
│   │   ├── type_.rs        # EntityType, RelationType, etc.
│   │   ├── instance.rs     # Entity, Relation, Attribute
│   │   └── value.rs        # Value, ValueType
│   ├── analyze/
│   │   ├── mod.rs          # AnalyzedQuery wrapper
│   │   ├── conjunction.rs
│   │   └── pipeline.rs
│   ├── common/
│   │   ├── mod.rs          # Error, Result, Promise, BoxStream, etc.
│   │   ├── error.rs        # Unified error type
│   │   ├── promise.rs      # Promise/BoxPromise (sync variant)
│   │   ├── stream.rs       # BoxStream (sync variant)
│   │   ├── query_options.rs
│   │   └── transaction_options.rs
│   └── user/
│       ├── mod.rs          # Stub user module
│       ├── user.rs         # User struct (stub)
│       └── user_manager.rs # UserManager (stub)
```

### lib.rs exports (must match remote driver)
```rust
pub use self::{
    common::{
        box_stream, error, BoxPromise, BoxStream, Error, Promise, QueryOptions, Result,
        TransactionOptions, TransactionType, IID,
    },
    database::{Database, DatabaseManager},
    driver::TypeDBDriver,
    transaction::Transaction,
    user::{User, UserManager},
};

// New embedded-specific exports
pub use self::driver::EmbeddedOptions;

pub mod analyze;
pub mod answer;
pub mod concept;
pub mod driver;
pub mod transaction;
```

---

## 7. Type Mapping: Engine Internals to Driver API

The engine and driver use different type hierarchies for concepts. The embedded driver must translate between them.

### Concept Mapping

| Driver Type | Engine Source | Translation |
|-------------|-------------|-------------|
| `driver::Concept::EntityType(EntityType)` | `answer::Type::Entity(concept::EntityType)` | Extract label from engine via `TypeManager::get_label()` |
| `driver::Concept::RelationType(RelationType)` | `answer::Type::Relation(concept::RelationType)` | Same |
| `driver::Concept::RoleType(RoleType)` | `answer::Type::RoleType(concept::RoleType)` | Same |
| `driver::Concept::AttributeType(AttributeType)` | `answer::Type::Attribute(concept::AttributeType)` | Extract label + value type |
| `driver::Concept::Entity(Entity)` | `answer::Thing::Entity(concept::Entity)` | Extract IID + type label |
| `driver::Concept::Relation(Relation)` | `answer::Thing::Relation(concept::Relation)` | Extract IID + type label |
| `driver::Concept::Attribute(Attribute)` | `answer::Thing::Attribute(concept::Attribute)` | Extract IID + type label + value |
| `driver::Concept::Value(Value)` | `encoding::value::Value` | Direct value conversion |

### Value Type Mapping

| Driver `ValueType` | Engine `encoding::value::ValueCategory` |
|--------------------|----------------------------------------|
| `Boolean` | `Boolean` |
| `Integer` | `Long` |
| `Double` | `Double` |
| `Decimal` | `Decimal` |
| `String` | `String` |
| `Date` | `Date` |
| `Datetime` | `DateTime` |
| `DatetimeTZ` | `DateTimeTZ` |
| `Duration` | `Duration` |
| `Struct(..)` | `Struct` |

### Query Result Mapping

The engine's query execution returns results in two forms:
- **Batch-based** (`executor::batch::Batch`): rows of fixed-width variable bindings
- **Document-based** (`executor::document::ConceptDocument`): tree-structured documents

Translation to driver API:

```
Engine Batch (rows of variable positions → answer::Concept)
    ↓ convert each row
Driver ConceptRow (column names + Vec<Concept>)
    ↓ wrap in stream
Driver QueryAnswer::ConceptRowStream(header, BoxStream<Result<ConceptRow>>)
```

```
Engine Vec<ConceptDocument>
    ↓ convert
Driver ConceptDocument (JSON-like tree)
    ↓ wrap in stream
Driver QueryAnswer::ConceptDocumentStream(header, BoxStream<Result<ConceptDocument>>)
```

The conversion layer lives in `answer/concept_row.rs` and `answer/concept_document.rs`. This is the most significant piece of new code — translating engine-internal `answer::Concept<'_>` (which holds lightweight type IDs and storage references) into the driver's self-contained `Concept` types (which hold materialized labels, values, and IIDs).

---

## 8. Transaction Lifecycle

### Read Transaction
```
TypeDBDriver::transaction("mydb", TransactionType::Read)
    → engine_dm.database("mydb")     // get Arc<Database<WALClient>>
    → TransactionRead::open(db, options)  // opens MVCC read snapshot
    → wrap in Transaction { inner: EmbeddedTxn::Read(txn_read) }
    → return Transaction

Transaction::query("match $x isa person;")
    → parse TypeQL
    → txn_read.query_manager.execute_read(snapshot, type_mgr, thing_mgr, func_mgr, pipeline)
    → convert Batch results to ConceptRow stream
    → return QueryAnswer::ConceptRowStream(..)

Transaction::close()
    → drop inner TransactionRead (releases MVCC snapshot)
```

### Write Transaction
```
TypeDBDriver::transaction("mydb", TransactionType::Write)
    → TransactionWrite::open(db, options)  // reserves write lock, opens write snapshot
    → wrap in Transaction { inner: EmbeddedTxn::Write(txn_write) }

Transaction::query("insert $x isa person;")
    → parse TypeQL
    → execute_write_query(txn_write, options, pipeline, source, interrupt)
    → return results

Transaction::commit()
    → txn_write.commit()  // validates isolation, writes WAL, applies to RocksDB
    → return Result
```

### Schema Transaction
```
TypeDBDriver::transaction("mydb", TransactionType::Schema)
    → TransactionSchema::open(db, options)  // reserves schema lock
    → wrap in Transaction { inner: EmbeddedTxn::Schema(txn_schema) }

Transaction::query("define person sub entity;")
    → parse TypeQL as SchemaQuery
    → execute_schema_query(txn_schema, query, source)
    → return QueryAnswer::Ok(QueryType::SchemaQuery)

Transaction::commit()
    → txn_schema.commit()  // validates, updates caches, writes WAL
```

### Internal Transaction Enum
```rust
enum EmbeddedTxn {
    Read(TransactionRead<WALClient>),
    Write(TransactionWrite<WALClient>),
    Schema(TransactionSchema<WALClient>),
}
```

---

## 9. Query Execution Path

### Current Engine Path (server-mediated)
```
gRPC request (TypeQL string)
    → server/service/grpc/ deserializes
    → database/query.rs dispatches by transaction type
    → query/query_manager.rs executes
    → compiler/ compiles TypeQL → IR
    → executor/ executes IR → Batch/ConceptDocument
    → server/service/grpc/ serializes to protobuf
    → gRPC response
```

### Embedded Path (direct)
```
Transaction::query(TypeQL string)
    → typeql::parse(query_str)       // parse to TypeQL AST
    → match transaction type:
        Read  → query_manager.execute_read(snapshot, type_mgr, thing_mgr, func_mgr, pipeline, options, interrupt)
        Write → database::query::execute_write_query(txn, options, pipeline, source, interrupt)
        Schema → database::query::execute_schema_query(txn, query, source)
    → convert engine results to driver QueryAnswer
    → return
```

The key difference: no serialization/deserialization, no protobuf, no network round-trip. The embedded driver calls the same `QueryManager` methods that the server's gRPC handler calls.

### ExecutionInterrupt

The engine uses `ExecutionInterrupt` for query cancellation. In the remote driver this is triggered by closing the transaction stream. In the embedded driver:

```rust
pub struct Transaction {
    inner: Mutex<Option<EmbeddedTxn>>,
    interrupt: ExecutionInterrupt,
    on_close_callbacks: Mutex<Vec<Box<dyn FnOnce(Option<Error>) + Send + Sync>>>,
}
```

`Transaction::close()` triggers the interrupt, which causes any in-progress query iterators to terminate.

---

## 10. Error Handling

### Engine Error Types
The engine uses `typedb_error!` macro-generated error types:
- `TransactionError`
- `DataCommitError`
- `SchemaCommitError`
- `QueryError`
- `DatabaseCreateError`
- `DatabaseOpenError`
- `DatabaseDeleteError`
- `ConceptReadError`
- `ConceptWriteError`
- `SnapshotError`

### Driver Error Type
The remote driver has a single `Error` enum:
```rust
pub enum Error {
    Connection(ConnectionError),
    Internal(InternalError),
    Migration(MigrationError),
    Other(String),
}
```

### Embedded Translation
The embedded driver maps engine errors to the driver's `Error` type:

```rust
impl From<TransactionError> for Error { .. }
impl From<Box<QueryError>> for Error { .. }
impl From<DataCommitError> for Error { .. }
impl From<SchemaCommitError> for Error { .. }
impl From<DatabaseCreateError> for Error { .. }
impl From<DatabaseDeleteError> for Error { .. }
impl From<DatabaseOpenError> for Error { .. }
```

`ConnectionError` variants are not used in embedded mode. Instead, engine-specific errors map to `Error::Internal` with the engine error's formatted message.

---

## 11. Async/Sync Duality

The remote driver supports both async and sync via:
- `#[cfg_attr(feature = "sync", maybe_async::must_be_sync)]` attribute
- Separate `promise_async.rs` / `promise_sync.rs` and `stream_async.rs` / `stream_sync.rs`

The embedded driver should maintain the same pattern for API compatibility:

```rust
#[cfg_attr(feature = "sync", maybe_async::must_be_sync)]
pub async fn transaction(&self, database_name: impl AsRef<str>, transaction_type: TransactionType) -> Result<Transaction> {
    // Engine calls are synchronous — wrap in spawn_blocking for async,
    // call directly for sync
    let db = self.engine_database_manager.database(database_name.as_ref())
        .ok_or_else(|| Error::database_not_found(database_name.as_ref()))?;

    let txn = match transaction_type {
        TransactionType::Read => EmbeddedTxn::Read(
            TransactionRead::open(db, self.default_txn_options())?
        ),
        TransactionType::Write => EmbeddedTxn::Write(
            TransactionWrite::open(db, self.default_txn_options())?
        ),
        TransactionType::Schema => EmbeddedTxn::Schema(
            TransactionSchema::open(db, self.default_txn_options())?
        ),
    };
    Ok(Transaction::new(txn))
}
```

**Important:** The engine's operations are synchronous (blocking I/O on RocksDB). In async mode, heavy operations (commit, large queries) should be wrapped in `tokio::task::spawn_blocking` to avoid blocking the async runtime.

---

## 12. Feature Flags

```toml
[features]
default = []
sync = []                     # Sync API (no async runtime needed)
diagnostics = ["diagnostics"] # Include TypeDB diagnostics/metrics
```

The `sync` feature mirrors the remote driver's `sync` feature, selecting between async and sync Promise/Stream implementations.

---

## 13. Dependencies

### Required (from engine)
```toml
database = { path = "../database" }
query = { path = "../query" }
compiler = { path = "../compiler" }
executor = { path = "../executor" }
ir = { path = "../ir" }
concept = { path = "../concept" }
function = { path = "../function" }
storage = { path = "../storage" }
durability = { path = "../durability" }
encoding = { path = "../encoding" }
answer = { path = "../answer" }
resource = { path = "../resource" }
options = { path = "../common/options" }
error = { path = "../common/error" }
logger = { path = "../common/logger" }
typeql = { git = "https://github.com/typedb/typeql", tag = "3.8.0" }
```

### Required (external)
```toml
tokio = { version = "1.47.1", features = ["rt", "sync", "macros", "time"] }
tracing = "0.1"
chrono = "0.4"
```

### Not required
- `tonic`, `prost`, `axum`, `axum-server` — no networking
- `rustls` — no TLS
- `sentry` — no crash reporting
- `clap` — no CLI
- `sysinfo` — no system introspection
- `typedb-protocol` — no protobuf serialization

---

## 14. Migration Path for Existing Users

### Cargo.toml Change
```diff
[dependencies]
- typedb-driver = "3.8"
+ typedb-embedded = { path = "../typedb/typedb-embedded" }
```

Or, when published:
```diff
- typedb-driver = "3.8"
+ typedb-embedded = "3.8"
```

### Code Changes

**Minimal — constructor only:**
```diff
- use typedb_driver::*;
+ use typedb_embedded::*;

- let driver = TypeDBDriver::new("localhost:1729", credentials, options).await?;
+ let driver = TypeDBDriver::new("./my_data_dir", EmbeddedOptions::default()).await?;
```

**Everything else stays the same:**
```rust
driver.databases().create("mydb").await?;
let tx = driver.transaction("mydb", TransactionType::Write).await?;
tx.query("insert $x isa person, has name 'Alice';").resolve()?;
tx.commit().resolve()?;
```

### Trait-Based Abstraction (optional, future)

For applications that want to support both embedded and remote modes, a shared `TypeDB` trait could be extracted:

```rust
pub trait TypeDB {
    type Db: TypeDBDatabase;
    type Txn: TypeDBTransaction;
    fn databases(&self) -> &dyn TypeDBDatabaseManager;
    async fn transaction(&self, db: &str, tt: TransactionType) -> Result<Self::Txn>;
}
```

This is **not** part of the initial design — it would be a future enhancement if demand exists.

---

## 15. Open Questions

1. **Constructor signature compatibility:** Option A (new `new_embedded()`) vs Option B (reinterpret `address` as `data_dir`). Recommendation: Option A for clarity, with a type alias `TypeDB = TypeDBDriver` for shorter usage.

2. **Tokio runtime management:** The engine's `DatabaseManager::new()` is synchronous. Should `typedb-embedded` create its own Tokio runtime internally (for spawn_blocking), or require the caller to provide one? Recommendation: in async mode, require a Tokio runtime context; in sync mode, create a minimal runtime internally.

3. **Schema export queries:** The remote driver's `Database::schema()` calls a server endpoint. The engine doesn't have a single "export schema as TypeQL" function. This needs implementation — iterate type definitions and format as TypeQL `define` statements.

4. **ConceptRow column names:** The engine's `Batch` uses `VariablePosition` indices. The driver's `ConceptRow` has named columns. The column name mapping comes from `PipelineStructure` (compiler output). Need to thread this through the result conversion.

5. **Crate publishing:** Should `typedb-embedded` live in the `typedb/typedb` repo (alongside the engine) or in `typedb/typedb-driver` (alongside the remote driver)? Recommendation: in `typedb/typedb` since it depends heavily on engine internals and should be versioned with the engine.

6. **Logging initialization:** The engine uses `tracing`. The embedded driver should not call `initialise_logging_global()` — let the host application configure its own tracing subscriber. Provide an optional `TypeDBDriver::init_logging(dir)` method for convenience.
