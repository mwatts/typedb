---
status: draft
---

# Extensibility: Pluggable Storage Backends and Custom Value Types

Two orthogonal extension axes for TypeDB:

1. **Storage Backend Abstraction** — decouple from RocksDB, enable redb (and others)
2. **Value Type Extensibility** — reduce the cost of adding types like `vector<N>`

These are designed to be **independent workstreams** that can proceed in parallel.
They share no code changes except at one narrow integration point (keyspace
descriptor metadata), documented below.

---

# PART 1: STORAGE BACKEND ABSTRACTION

## Current State

RocksDB is well-encapsulated. Consumer code (71+ files across executor, concept,
compiler, query) interacts only through snapshot traits:

```
Consumer code  →  ReadableSnapshot / WritableSnapshot / CommittableSnapshot
                          ↓
                  MVCCStorage<D: DurabilityClient>
                          ↓
                  Keyspaces → Vec<Keyspace>  (one RocksDB DB per keyspace)
                          ↓
                  rocksdb::DB / rocksdb::DBRawIterator
```

**RocksDB import locations outside storage/:**

| Crate | File | What it imports | Why |
|-------|------|-----------------|-----|
| `encoding` | `encoding/encoding.rs` | `BlockBasedOptions`, `DBCompressionType`, `SliceTransform` | `EncodingKeyspace::rocks_configuration()` |
| `common/cache` | `Cargo.toml` | `rocksdb` crate dependency | Unclear — needs investigation |

Everything else (database, server, executor, concept, compiler, query) uses only
the snapshot traits. **No RocksDB types leak to consumers.**

**Custom MVCC layer:** TypeDB builds its own MVCC on top of RocksDB using
inverted sequence numbers appended to keys, an `IsolationManager`, and a
`Timeline`. RocksDB is used as a dumb ordered byte store.

## Target Architecture

```
Consumer code  →  ReadableSnapshot / WritableSnapshot / CommittableSnapshot  (UNCHANGED)
                          ↓
                  MVCCStorage<B: StorageBackend, D: DurabilityClient>
                          ↓
                  StorageBackend trait  ← NEW
                    ├── RocksDbBackend  (extracted from current code)
                    └── RedbBackend    (new implementation)
```

## Design

### 1.1 StorageBackend Trait

```rust
// storage/backend/mod.rs

/// Minimal trait for a sorted key-value store backend.
///
/// The backend is unaware of MVCC — it stores and retrieves opaque byte keys
/// and values. MVCC key encoding (sequence numbers, operation tags) happens
/// in the layer above.
pub trait StorageBackend: Send + Sync + 'static {
    type Config: Send + Sync + Clone;
    type Keyspace: KeyspaceOps + Send + Sync;
    type WriteBatch: BackendWriteBatch + Send;

    /// Open or create a database at the given path with the specified keyspaces.
    fn open(path: &Path, keyspaces: &[KeyspaceDescriptor], config: &Self::Config)
        -> Result<Self, StorageOpenError>
    where
        Self: Sized;

    /// Get a handle to a specific keyspace by ID.
    fn keyspace(&self, id: KeyspaceId) -> &Self::Keyspace;

    /// Create a checkpoint (for backup/recovery).
    fn checkpoint(&self, path: &Path) -> Result<(), CheckpointCreateError>;

    /// Estimated total size on disk.
    fn estimated_size_bytes(&self) -> u64;

    /// Delete a database at the given path.
    fn delete(path: &Path) -> Result<(), StorageDeleteError>
    where
        Self: Sized;
}
```

### 1.2 KeyspaceOps Trait

```rust
// storage/backend/mod.rs

pub trait KeyspaceOps: Send + Sync {
    /// Point lookup. Returns owned bytes.
    fn get(&self, key: &[u8]) -> Result<Option<ByteArray<BUFFER_VALUE_INLINE>>, KeyspaceError>;

    /// Point lookup with zero-copy callback. The callback receives the value
    /// bytes while the internal lock/pin is held.
    fn get_mapped<T>(
        &self,
        key: &[u8],
        f: impl FnOnce(&[u8]) -> T,
    ) -> Result<Option<T>, KeyspaceError>;

    /// Seek to the largest key <= the given key and return it.
    /// Used by MVCC to find the latest visible version.
    fn get_prev(&self, key: &[u8]) -> Result<Option<(ByteArray<48>, ByteArray<48>)>, KeyspaceError>;

    /// Forward range iteration. The returned iterator yields (key, value) pairs
    /// in ascending byte order.
    fn iterate_range<'a>(
        &'a self,
        range: KeyRange<'_>,
        pool: &'a IteratorPool,
    ) -> impl BackendIterator + 'a;

    /// Create a new write batch for atomic multi-key writes.
    fn new_write_batch(&self) -> impl BackendWriteBatch;

    /// Apply a write batch atomically.
    fn write(&self, batch: impl BackendWriteBatch) -> Result<(), KeyspaceError>;
}
```

### 1.3 BackendIterator Trait

```rust
// storage/backend/iterator.rs

/// A lending iterator over sorted key-value pairs.
///
/// This follows the same pattern as the current MVCCRangeIterator: peek
/// returns a reference valid until the next call to advance().
pub trait BackendIterator {
    fn peek(&mut self) -> Option<Result<(&[u8], &[u8]), BackendReadError>>;
    fn advance(&mut self);
    fn seek(&mut self, key: &[u8]);
}

pub trait BackendWriteBatch {
    fn put(&mut self, key: &[u8], value: &[u8]);
    fn delete(&mut self, key: &[u8]);
    fn len(&self) -> usize;
}
```

### 1.4 KeyspaceDescriptor (Replaces KeyspaceSet)

The current `KeyspaceSet` trait has a `rocks_configuration()` method that returns
`rocksdb::Options`. This is the **only place** outside `storage/` that imports
RocksDB types (in `encoding/encoding.rs`).

Replace with a backend-agnostic descriptor:

```rust
// storage/backend/mod.rs

/// Backend-agnostic description of a keyspace's access patterns.
/// Backends use this to configure their internal optimizations
/// (bloom filters, prefix extractors, page sizes, etc.)
#[derive(Clone, Debug)]
pub struct KeyspaceDescriptor {
    pub id: KeyspaceId,
    pub name: &'static str,
    /// Common prefix length for keys in this keyspace.
    /// Backends can use this for bloom filter / prefix extractor configuration.
    pub prefix_length: Option<usize>,
    /// Whether all keys have the same length.
    pub fixed_width_keys: bool,
}
```

The `EncodingKeyspace` enum stays, but its `KeyspaceSet` implementation becomes:

```rust
impl EncodingKeyspace {
    pub fn descriptor(&self) -> KeyspaceDescriptor {
        KeyspaceDescriptor {
            id: self.id(),
            name: self.name(),
            prefix_length: Some(self.prefix_length()),
            fixed_width_keys: false,
        }
    }
}
```

Backend-specific tuning moves into each backend's `Config` type:

```rust
pub struct RocksDbConfig {
    pub cache_size_mb: usize,
    pub compression_levels: Vec<CompressionType>,
    pub target_file_size_base: u64,
    pub write_buffer_size: u64,
    pub max_write_buffer_number: i32,
    pub max_background_jobs: i32,
}

pub struct RedbConfig {
    pub cache_size_mb: usize,
}
```

### 1.5 RocksDB Backend (Extraction)

Extract the current implementation from:

| Current file | Extracted to |
|-------------|-------------|
| `storage/keyspace/keyspace.rs` (Keyspace struct) | `storage/backend/rocksdb/keyspace.rs` |
| `storage/keyspace/raw_iterator.rs` | `storage/backend/rocksdb/iterator.rs` |
| `storage/keyspace/mod.rs` (IteratorPool) | `storage/backend/rocksdb/iterator_pool.rs` |
| `encoding/encoding.rs` (rocks_configuration) | `storage/backend/rocksdb/config.rs` |

The `Keyspaces` struct dissolves — `RocksDbBackend` holds a `Vec<RocksDbKeyspace>`
directly.

The `unsafe transmute` in `raw_iterator.rs` (for the lending iterator lifetime)
stays in the RocksDB backend. The redb backend won't need it.

### 1.6 redb Backend

Key mapping from RocksDB concepts to redb:

| RocksDB concept | redb equivalent |
|----------------|----------------|
| Separate DB per keyspace | Named table per keyspace in one `Database` |
| `DBRawIterator` (unsafe, lending) | `Range<&[u8], &[u8]>` (safe, owned) |
| `WriteBatch` (applied atomically) | `WriteTransaction` (ACID built-in) |
| `seek_for_prev()` | `range(..=key).rev().next()` |
| `Checkpoint` | `Database::compact()` + file copy |
| `SliceTransform` prefix bloom | Not needed — redb uses B-tree, prefix scan is efficient by default |
| Column family options | Per-table configuration (minimal) |

MVCC decision: **keep TypeDB's custom MVCC** for the initial implementation.
redb's built-in ACID transactions are used only for write batch atomicity. This
minimizes risk — the isolation semantics are identical regardless of backend.

A future optimization could leverage redb's native snapshot isolation to
eliminate the MVCC key overhead, but that's a separate project.

### 1.7 IteratorPool Handling

The current `IteratorPool` recycles `DBRawIterator` instances to avoid allocation.
This is a RocksDB-specific optimization (creating iterators is expensive due to
memtable pinning).

For the trait, make pooling backend-internal:

```rust
// In BackendIterator creation, the backend handles its own pooling.
// RocksDbBackend maintains IteratorPool internally.
// RedbBackend doesn't need pooling (redb iterators are cheap).
```

The `IteratorPool` type moves from `storage/keyspace/mod.rs` into
`storage/backend/rocksdb/`. The snapshot layer no longer needs to pass pools
around — the backend keyspace handle manages its own resources.

### 1.8 Migration Plan

**Phase S1: Define traits (no behavior change)**
- Create `storage/backend/mod.rs` with `StorageBackend`, `KeyspaceOps`,
  `BackendIterator`, `BackendWriteBatch` traits
- Create `KeyspaceDescriptor` struct
- Compile check: everything still builds

**Phase S2: Extract RocksDB backend**
- Move keyspace code to `storage/backend/rocksdb/`
- Implement `StorageBackend` for `RocksDbBackend`
- Move `rocks_configuration()` from `encoding/encoding.rs` into
  `storage/backend/rocksdb/config.rs`
- Remove `rocksdb` dependency from `encoding` crate
- `EncodingKeyspace` now only provides `KeyspaceDescriptor`

**Phase S3: Make MVCCStorage generic**
- `MVCCStorage<B: StorageBackend, D: DurabilityClient>`
- Replace `Keyspaces` field with `B`
- Update snapshot types to carry `B` generic
- Update `database.rs` to instantiate with `RocksDbBackend`
- All consumer code unchanged (still uses trait objects / snapshot traits)

**Phase S4: Implement redb backend**
- `storage/backend/redb/mod.rs`
- Implement all traits
- Integration tests comparing both backends with identical workloads

**Phase S5: Runtime backend selection**
- Database creation accepts backend choice
- Existing databases auto-detect their backend from metadata
- Config flag / CLI option for default backend

### 1.9 Risk Assessment

| Risk | Mitigation |
|------|-----------|
| `BackendIterator` lending lifetime is hard to express in traits | Use GAT (`type Iter<'a>: ... + 'a where Self: 'a`) or box the iterator. Prefer GAT for zero-cost on RocksDB path |
| `get_prev()` semantics differ between backends | Specify contract precisely. redb's `range(..=key).rev().next()` is semantically equivalent |
| Performance regression from trait indirection | Use monomorphization (`impl StorageBackend` in generic code, not `dyn`). Zero-cost at compile time |
| IteratorPool removal from snapshot layer | RocksDB backend manages pools internally. No change in pool behavior, just ownership |

---

# PART 2: VALUE TYPE EXTENSIBILITY

## Current State

Adding a new value type (e.g. `vector<N>`) requires changes in 20+ files across
6 layers due to exhaustive `match` on `ValueType`, `ValueTypeCategory`,
`AttributeID`, and `Value` enums.

**Exhaustive match sites by layer:**

| Layer | Enum matched | Approx. match sites | Files |
|-------|-------------|---------------------|-------|
| Encoding | `ValueType`, `ValueTypeCategory`, `AttributeID` | 15+ | `encoding/value/`, `encoding/graph/thing/` |
| IR | `ValueType` (IR variant) | 5+ | `ir/pattern/`, `ir/translation/` |
| Compiler | `ValueTypeCategory`, comparison tables | 10+ | `compiler/annotation/expression/` |
| Executor | `Value`, instruction dispatch | 10+ | `executor/instruction/`, `executor/read/` |
| Concept | `Value`, type manager | 5+ | `concept/thing/` |
| Server/Protocol | `ValueType` ↔ protobuf conversion | 3+ | `server/service/grpc/` |

## External Repo Dependencies (Problematic Complexity)

Adding a value type is **not contained to this repo**. These external repos
must also change:

### typeql (github.com/typedb/typeql, tag 3.8.0)
- **TypeQL parser** — must parse new type syntax (e.g. `vector(768)`)
- **TypeQL AST** — `ValueType` enum in the TypeQL crate. This is the source
  of truth that IR translation consumes.
- **13 crates** in this repo depend on `typeql`. All will see the new AST types.
- **Impact**: Must be changed first. New TypeDB server version requires new
  TypeQL version.

### typedb-protocol (github.com/typedb/typedb-protocol, tag 3.7.0)
- **Protobuf definitions** — `ValueType` message in the wire protocol.
  Currently has variants: Boolean, Integer, Double, Decimal, Date, DateTime,
  DateTimeTZ, Duration, String, Struct.
- **gRPC encode/decode** — `server/service/grpc/concept.rs:176-203` matches
  `ValueType` → protobuf. Adding a variant here requires a new protobuf
  message type.
- **Impact**: Protocol version bump required. All drivers (Java, Python, Node,
  Rust, etc.) must be updated.
- **Backward compatibility**: Protobuf oneof is forward-compatible (old clients
  ignore unknown variants), but old servers reject unknown types.

### typedb drivers (multiple repos)
- Each driver has its own `ValueType` mapping.
- Must be updated to handle the new protobuf variant.
- **Impact**: N driver repos, each with their own release cycle.

### Coordination problem
Adding `vector` today requires synchronized releases of:
1. `typeql` (parser + AST)
2. `typedb-protocol` (wire format)
3. `typedb` (this repo — IR, compiler, encoding, executor, server)
4. All driver repos

The extensibility design below aims to make steps 1 and 3 cheaper (fewer files
touched, guided by traits instead of grep). Steps 2 and 4 are inherently
cross-repo but can be mitigated with a protocol extension mechanism.

## Design

### 2.1 ValueTypeDescriptor Trait

Replace per-type match arms with a trait that each value type implements:

```rust
// encoding/value/type_descriptor.rs

/// Describes the storage and comparison properties of a value type.
/// Each value type (boolean, integer, double, ..., vector) implements this.
pub trait ValueTypeDescriptor: Send + Sync + 'static {
    /// Unique byte tag for storage encoding. Must be stable across versions.
    fn category_id(&self) -> u8;

    /// Human-readable name (e.g. "boolean", "vector").
    fn name(&self) -> &'static str;

    /// Whether values of this type can be stored in B-tree index keys
    /// (i.e., have a total byte ordering that matches semantic ordering).
    fn is_keyable(&self) -> bool;

    /// Fixed encoding length in bytes, or None for variable-length.
    fn encoding_length(&self) -> Option<usize>;

    /// Which other value types this type can be compared to.
    fn is_comparable_to(&self, other_category_id: u8) -> bool;

    /// Whether this type can be implicitly cast to another.
    fn is_castable_to(&self, other_category_id: u8) -> bool;

    /// Encode a value to bytes for storage.
    fn encode_value(&self, value: &Value, buf: &mut BytesMut);

    /// Decode a value from storage bytes.
    fn decode_value(&self, bytes: &[u8]) -> Value;

    /// Compare two encoded values in storage byte form.
    /// Returns None if the type has no natural ordering (e.g., vectors).
    fn compare_encoded(&self, a: &[u8], b: &[u8]) -> Option<Ordering>;
}
```

### 2.2 ValueTypeRegistry

```rust
// encoding/value/type_registry.rs

/// Central registry mapping category IDs and names to descriptors.
/// Built once at startup, immutable thereafter.
pub struct ValueTypeRegistry {
    by_id: [Option<&'static dyn ValueTypeDescriptor>; 256],
    by_name: HashMap<&'static str, &'static dyn ValueTypeDescriptor>,
}

impl ValueTypeRegistry {
    pub fn builder() -> ValueTypeRegistryBuilder { ... }
    pub fn get_by_id(&self, id: u8) -> Option<&dyn ValueTypeDescriptor> { ... }
    pub fn get_by_name(&self, name: &str) -> Option<&dyn ValueTypeDescriptor> { ... }
    pub fn all(&self) -> impl Iterator<Item = &dyn ValueTypeDescriptor> { ... }
}
```

Built at startup with all known types:

```rust
let registry = ValueTypeRegistry::builder()
    .register(BooleanType)
    .register(IntegerType)
    .register(DoubleType)
    .register(DecimalType)
    .register(DateType)
    .register(DateTimeType)
    .register(DateTimeTZType)
    .register(DurationType)
    .register(StringType)
    .register(StructType)
    // New types just add a line:
    .register(VectorType { max_dims: 65536 })
    .build();
```

### 2.3 AttributeStorageStrategy

Currently `AttributeID` is an enum with 10 variants (one per type), each with
different inline buffer sizes. Replace with a uniform representation:

```rust
// encoding/graph/thing/attribute_id.rs

/// Uniform attribute ID — type tag + encoded value bytes.
/// Replaces the 10-variant AttributeID enum.
pub struct AttributeID {
    /// Which value type category this attribute stores.
    category_id: u8,
    /// Encoded value bytes (inline for small values, heap for large).
    bytes: ByteArray<BUFFER_VALUE_INLINE>,
}

impl AttributeID {
    pub fn build(value: &Value, registry: &ValueTypeRegistry) -> Self { ... }
    pub fn extract_value(&self, registry: &ValueTypeRegistry) -> Value { ... }
    pub fn category_id(&self) -> u8 { self.category_id }
}
```

### 2.4 Expression Operation Registry

```rust
// compiler/annotation/expression/op_registry.rs

/// Defines what operations a value type supports in expressions.
pub trait ValueTypeOperations: Send + Sync + 'static {
    fn category_id(&self) -> u8;

    /// Binary operations this type supports (add, sub, mul, div, mod, power).
    fn compile_binary_op(
        &self,
        op: ArithmeticOp,
        rhs_category_id: u8,
        ctx: &mut ExpressionCompilationContext,
    ) -> Result<ExpressionValueType, ExpressionCompileError>;

    /// Comparison operations (=, !=, <, >, <=, >=).
    fn compile_comparison(
        &self,
        op: ComparisonOp,
        rhs_category_id: u8,
        ctx: &mut ExpressionCompilationContext,
    ) -> Result<ExpressionValueType, ExpressionCompileError>;

    /// Built-in unary functions (abs, ceil, floor, round, len).
    fn compile_unary_function(
        &self,
        func: UnaryFunctionOp,
        ctx: &mut ExpressionCompilationContext,
    ) -> Result<ExpressionValueType, ExpressionCompileError>;
}
```

For vectors, the implementation returns `Err(UnsupportedOperation)` for
arithmetic and comparison, while registering custom functions like
`cosine_distance` through the function registry.

### 2.5 Index Strategy Trait

```rust
// compiler/executable/match_/planner/index_strategy.rs

/// A pluggable index that the query planner can consider.
pub trait IndexStrategy: Send + Sync + 'static {
    /// Human-readable name for diagnostics.
    fn name(&self) -> &str;

    /// Whether this index can accelerate the given constraint pattern.
    fn can_serve(
        &self,
        constraint: &Constraint,
        type_annotations: &TypeAnnotations,
    ) -> bool;

    /// Estimated cost of using this index with the given bound variables.
    fn estimate_cost(
        &self,
        constraint: &Constraint,
        bound_variables: &BTreeSet<Variable>,
        statistics: &Statistics,
    ) -> f64;

    /// Create an executable step for this index.
    fn create_executor(
        &self,
        constraint: &Constraint,
        variable_modes: &VariableModes,
    ) -> Box<dyn IndexExecutor>;
}
```

This enables:
- **BTreePrefixScanIndex** — existing behavior, works on any `StorageBackend`
- **RelationBinaryIndex** — existing `IndexedRelation`, extracted to this trait
- **VectorANNIndex** — new, wraps usearch/faiss, responds to distance predicates

### 2.6 Protocol Extension Mechanism

For `typedb-protocol`, instead of adding a protobuf variant per new type,
add a generic extension:

```protobuf
// In typedb-protocol value_type.proto

message ValueType {
    oneof value_type {
        Boolean boolean = 1;
        Integer integer = 2;
        // ... existing types ...
        Struct struct = 10;
        // Extension point for new types:
        ExtensionType extension = 15;
    }
}

message ExtensionType {
    string type_name = 1;          // e.g. "vector"
    map<string, string> params = 2; // e.g. {"dims": "768"}
}
```

This allows new value types to be transmitted over the wire without a protocol
version bump. Drivers that don't know about a type can still display it
generically. Drivers that do know about it can provide native support.

### 2.7 TypeQL Integration Strategy

TypeQL (the external parser crate) defines the `ValueType` enum that the IR
translation layer consumes. Two approaches:

**Option A: Extend TypeQL's ValueType enum**
- Add `Vector(u32)` variant to TypeQL's `ValueType`
- Requires TypeQL release before TypeDB release
- Clean but tight coupling

**Option B: Add a generic extension variant to TypeQL**
```rust
// In typeql crate
pub enum ValueType {
    Boolean,
    Integer,
    // ... existing ...
    Extension { name: String, params: Vec<TypeParam> },
}
```
- One TypeQL change, then new types don't need TypeQL changes
- TypeDB's IR translation maps `Extension { name: "vector", .. }` to the
  registered `VectorType` descriptor
- Looser coupling, but TypeQL can't validate type-specific syntax

**Recommendation**: Option B for extensibility, with TypeQL providing parse
support for the `extension(name, params...)` syntax once.

### 2.8 Migration Plan

**Phase V1: ValueTypeDescriptor + Registry (no behavior change)**
- Define `ValueTypeDescriptor` trait
- Implement for all 10 existing types
- Build `ValueTypeRegistry` at startup
- Existing match sites still work; new code can use registry

**Phase V2: AttributeID unification**
- Replace `AttributeID` 10-variant enum with uniform struct
- Update encoding/decoding to use `ValueTypeDescriptor::encode_value/decode_value`
- Eliminates ~10 match sites in encoding layer

**Phase V3: Expression operation registry**
- Define `ValueTypeOperations` trait
- Implement for existing types
- Refactor expression compiler to dispatch through registry
- Eliminates ~10 match sites in compiler

**Phase V4: Index strategy trait**
- Define `IndexStrategy` trait
- Extract existing `IndexedRelation` into it
- Planner consults registered strategies
- Enables future ANN index

**Phase V5: Protocol extension + TypeQL extension variant**
- Add `ExtensionType` to protocol
- Add extension variant to TypeQL (requires external repo PR)
- Update server encode/decode to handle extensions via registry

---

# ORTHOGONALITY

The two workstreams are intentionally independent:

```
                    Storage Backend Abstraction (Part 1)
                    ====================================
                    Touches: storage/, encoding/encoding.rs (keyspace config only)
                    Does NOT touch: ValueType, Value, AttributeID, expressions,
                                    compiler, executor instruction dispatch

                    Value Type Extensibility (Part 2)
                    =================================
                    Touches: encoding/value/, ir/pattern/, compiler/annotation/,
                             executor/instruction/, server/service/grpc/
                    Does NOT touch: storage backend, keyspace, iterators, MVCC,
                                    durability, snapshots
```

**Shared touch point:** `encoding/encoding.rs` — Part 1 removes
`rocks_configuration()` from here, Part 2 doesn't touch this file. No conflict.

**Dependency:** `AttributeStorageStrategy` (Part 2) calls methods on whatever
storage backend is active (Part 1), but only through the existing
`ReadableSnapshot` / `WritableSnapshot` traits which both workstreams preserve.

**Recommended sequencing:** Either can go first. If working in parallel, the
only coordination needed is that both agree on the `KeyspaceDescriptor` struct
(Part 1 defines it, Part 2 doesn't modify it).

---

# IMPLEMENTATION PRIORITY MATRIX

| Phase | Workstream | Effort | Files changed | Risk |
|-------|-----------|--------|---------------|------|
| S1 | Storage: define traits | S | 2-3 new files | None |
| S2 | Storage: extract RocksDB | M | ~8 files moved/refactored | Low |
| V1 | Value: descriptor + registry | S | ~5 new files + startup wiring | None |
| S3 | Storage: generic MVCCStorage | L | ~15 files (storage/, database/) | Medium |
| V2 | Value: AttributeID unification | M | ~10 files in encoding/ | Low |
| V3 | Value: expression op registry | M | ~10 files in compiler/ | Low |
| S4 | Storage: redb implementation | M | ~4 new files + integration tests | Medium |
| V4 | Value: index strategy trait | L | ~8 files in compiler/executor | Medium |
| V5 | Value: protocol + TypeQL ext | M | External repo PRs required | Medium |
| S5 | Storage: runtime selection | S | database.rs + CLI config | Low |

S = small (1-3 days), M = medium (3-7 days), L = large (1-2 weeks)

---

# APPENDIX: FILE INVENTORY

## Files that change for Storage Backend Abstraction

**New files:**
- `storage/backend/mod.rs` — traits
- `storage/backend/rocksdb/mod.rs` — extracted backend
- `storage/backend/rocksdb/keyspace.rs` — extracted from `storage/keyspace/keyspace.rs`
- `storage/backend/rocksdb/iterator.rs` — extracted from `storage/keyspace/raw_iterator.rs`
- `storage/backend/rocksdb/iterator_pool.rs` — extracted from `storage/keyspace/mod.rs`
- `storage/backend/rocksdb/config.rs` — extracted from `encoding/encoding.rs`
- `storage/backend/redb/mod.rs` — new backend

**Modified files:**
- `storage/storage.rs` — `MVCCStorage` becomes generic over `B: StorageBackend`
- `storage/snapshot/*.rs` — snapshot types carry `B` generic
- `storage/iterator.rs` — `MVCCRangeIterator` wraps `B::Iterator` instead of `DBRawIterator`
- `encoding/encoding.rs` — remove `rocks_configuration()`, keep `KeyspaceDescriptor`
- `encoding/Cargo.toml` — remove `rocksdb` dependency
- `database/database.rs` — instantiate with `RocksDbBackend`
- `database/transaction.rs` — carry `B` generic

**Unchanged:** All consumer code (executor, concept, compiler, query, server, function)

## Files that change for Value Type Extensibility

**New files:**
- `encoding/value/type_descriptor.rs` — trait + registry
- `encoding/value/types/*.rs` — one file per type implementing descriptor
- `compiler/annotation/expression/op_registry.rs` — expression operations trait

**Modified files:**
- `encoding/value/mod.rs` — wire in registry
- `encoding/graph/thing/vertex_attribute.rs` — unified `AttributeID`
- `ir/pattern/mod.rs` — IR `ValueType` gains extension variant
- `ir/translation/constraints.rs` — use registry for type resolution
- `compiler/annotation/type_seeder.rs` — use descriptor for compatibility
- `compiler/annotation/expression/expression_compiler.rs` — dispatch via registry
- `compiler/executable/match_/planner/` — consult `IndexStrategy` registry
- `server/service/grpc/concept.rs` — encode via registry + extension type
- `server/service/grpc/analyze.rs` — same

**External repos requiring changes:**
- `typeql` — add extension value type variant to AST
- `typedb-protocol` — add `ExtensionType` message to protobuf
- Driver repos — handle `ExtensionType` in deserialization
