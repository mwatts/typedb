---
status: draft
---

# Extensibility: Pluggable Storage Backends and Custom Value Types

Two orthogonal extension axes for TypeDB:

1. **Storage Backend Abstraction** — decouple from RocksDB, enable alternative backends
2. **Value Type Extensibility** — reduce the cost of adding types like `vector<N>`

These are designed to be **independent workstreams** that can proceed in parallel.
They share no code changes except at one narrow integration point (keyspace
descriptor metadata), documented below.

---

# PART 1: STORAGE BACKEND ABSTRACTION

## Completed: The `kv` Crate (Enum Dispatch)

The `replaceable-storage` branch introduced the `kv/` crate, which provides a
backend-agnostic key-value interface using **enum dispatch**. This is now merged
and is the foundation for all future backend work.

### Architecture

```
Consumer code  →  ReadableSnapshot / WritableSnapshot / CommittableSnapshot  (UNCHANGED)
                          ↓
                  MVCCStorage<D: DurabilityClient>
                          ↓
                  Keyspaces → Vec<KVStore>   (kv/ crate)
                          ↓
                  KVStore enum dispatch
                    └── KVStore::RocksDB(RocksKVStore)    (kv/rocks/)
                    └── (future: InMemory, Redb, WASM, etc.)
```

### Key Design Decisions

Three approaches were evaluated:

| Approach | Pros | Cons | Decision |
|----------|------|------|----------|
| **Trait-based** (`trait KVStore`) | Conceptually cleanest | Generic parameter propagates everywhere — huge surface area in `//concept`, `//traversal`, etc. | Rejected |
| **Trait objects** (`dyn KVStore`) | Keeps strong typing hidden in `//storage` | Many required traits (`KVStore`, iterators) are not object-safe. Requires `Any` downcasting workarounds. | Rejected |
| **Enum dispatch** | Zero-cost for single-backend builds. No generic propagation. Scales to N backends cleanly. Feature-flag friendly. | Match arms grow linearly with backends (acceptable). | **Chosen** |

### What Was Implemented

**New `kv/` crate** with these core types:

```rust
// kv/lib.rs — Enum dispatch for KV backends
pub enum KVStore {
    RocksDB(RocksKVStore),
    // Future variants: InMemory(...), Redb(...), etc.
}

impl KVStore {
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn TypeDBError>>;
    pub fn get<M, V>(&self, key: &[u8], mapper: M) -> Result<Option<V>, Box<dyn TypeDBError>>;
    pub fn get_prev<M, T>(&self, key: &[u8], mapper: M) -> Option<T>;
    pub fn iterate_range(&self, range: &KeyRange<...>, counters: StorageCounters) -> KVRangeIterator;
    pub fn write(&self, write_batch: KVWriteBatch) -> Result<(), Box<dyn TypeDBError>>;
    pub fn checkpoint(&self, checkpoint_dir: &Path) -> Result<(), Box<dyn TypeDBError>>;
    pub fn delete(self) -> Result<(), Box<dyn TypeDBError>>;
    pub fn reset(&mut self) -> Result<(), Box<dyn TypeDBError>>;
    pub fn estimate_size_in_bytes(&self) -> Result<u64, Box<dyn TypeDBError>>;
    pub fn estimate_key_count(&self) -> Result<u64, Box<dyn TypeDBError>>;
}
```

```rust
// kv/iterator.rs — Enum dispatch for iterators
pub enum KVRangeIterator {
    RocksDB(RocksRangeIterator),
}
impl LendingIterator for KVRangeIterator { ... }
impl Seekable<[u8]> for KVRangeIterator { ... }
```

```rust
// kv/write_batches.rs — Enum dispatch for write batches
pub enum KVWriteBatch {
    RocksDB(rocksdb::WriteBatch),
}

pub struct WriteBatches {
    pub batches: [Option<KVWriteBatch>; KEYSPACE_MAXIMUM_COUNT],
}
```

```rust
// kv/keyspaces.rs — Backend-agnostic keyspace management
pub trait KeyspaceSet: Copy {
    fn iter() -> impl Iterator<Item = Self>;
    fn id(&self) -> KeyspaceId;
    fn name(&self) -> &'static str;
    fn prefix_length(&self) -> Option<usize>;
}

pub struct Keyspaces {
    keyspaces: Vec<KVStore>,
    index: [Option<KeyspaceId>; KEYSPACE_MAXIMUM_COUNT],
}
```

**RocksDB backend** in `kv/rocks/`:
- `kv/rocks/mod.rs` — `RocksKVStore` struct with all RocksDB-specific configuration
- `kv/rocks/iterator.rs` — `RocksRangeIterator` wrapping `DBRawIterator`
- `kv/rocks/iterpool.rs` — Raw iterator pool (RocksDB-specific optimization)
- `kv/rocks/pool.rs` — Connection pool

### What Was Removed or Moved

| Before | After | Notes |
|--------|-------|-------|
| `storage/keyspace/keyspace.rs` (420 lines) | `kv/rocks/mod.rs` | RocksDB logic extracted |
| `storage/keyspace/raw_iterator.rs` (158 lines) | `kv/rocks/iterator.rs` | Iterator extracted |
| `storage/keyspace/mod.rs` (47 lines) | `kv/keyspaces.rs` | Keyspace management moved |
| `storage/keyspace/constants.rs` | `kv/keyspaces.rs` | Constants inlined |
| `storage/write_batches.rs` (83 lines) | `kv/write_batches.rs` | Write batch types moved |
| `storage/key_range.rs` | `common/primitive/key_range.rs` | Not storage-specific |
| `encoding/encoding.rs` `rocks_configuration()` | `kv/rocks/mod.rs` `create_open_options()` | RocksDB config no longer leaks |

### Current RocksDB Import Surface

After the refactoring, RocksDB imports are contained to:

| Location | What | Why |
|----------|------|-----|
| `kv/rocks/*` | All RocksDB types | The backend implementation |
| `kv/write_batches.rs` | `rocksdb::WriteBatch` | Inside `KVWriteBatch::RocksDB` variant |
| `storage/benches/` | Direct RocksDB for benchmarking | Test-only |

**No consumer crate** (encoding, concept, compiler, executor, query, server,
function, ir, database) imports RocksDB types. The `encoding` crate depends on
`kv` only for `KeyspaceSet`/`KeyspaceId` — purely structural, no backend types.

### Impact Summary

- **73 files changed** across 76 modules
- **Generic parameter `KV` removed** from storage, encoding, concept, database,
  function, ir, and all higher-level crates
- `MVCCStorage<D>` is now generic only over `DurabilityClient`, not the backend
- All consumer code interacts through snapshot traits — unchanged

## Remaining Storage Work

### Phase S1: Add In-Memory Backend (Testing)

Add a simple in-memory `BTreeMap`-based backend for faster unit tests and
WASM compatibility exploration:

```rust
// kv/lib.rs
pub enum KVStore {
    RocksDB(RocksKVStore),
    InMemory(InMemoryKVStore),  // NEW
}
```

- `kv/memory/mod.rs` — `InMemoryKVStore` backed by `BTreeMap<Vec<u8>, Vec<u8>>`
- `kv/memory/iterator.rs` — Simple range iterator over BTreeMap
- Enable via feature flag: `#[cfg(feature = "memory-backend")]`

### Phase S2: Add redb Backend

Key mapping from RocksDB concepts to redb:

| RocksDB concept | redb equivalent |
|----------------|----------------|
| Separate DB per keyspace | Named table per keyspace in one `Database` |
| `DBRawIterator` (unsafe, lending) | `Range<&[u8], &[u8]>` (safe, owned) |
| `WriteBatch` (applied atomically) | `WriteTransaction` (ACID built-in) |
| `seek_for_prev()` | `range(..=key).rev().next()` |
| `Checkpoint` | `Database::compact()` + file copy |
| Bloom filter prefix extractor | Not needed — redb uses B-tree |

MVCC decision: **keep TypeDB's custom MVCC** for the initial implementation.
redb's built-in ACID transactions are used only for write batch atomicity.

A future optimization could leverage redb's native snapshot isolation to
eliminate the MVCC key overhead, but that's a separate project.

### Phase S3: Runtime Backend Selection

- Database creation accepts backend choice
- Existing databases auto-detect their backend from metadata
- Config flag / CLI option for default backend
- Could even allow different backends per-database

### Phase S4: Per-Database Backend Configuration

The enum dispatch approach naturally supports this since each `KVStore` instance
is independent. Different databases could use different backends without any
architectural changes.

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
- **BTreePrefixScanIndex** — existing behavior, works on any backend
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
                    Touches: kv/, storage/, common/primitive/
                    Does NOT touch: ValueType, Value, AttributeID, expressions,
                                    compiler, executor instruction dispatch

                    Value Type Extensibility (Part 2)
                    =================================
                    Touches: encoding/value/, ir/pattern/, compiler/annotation/,
                             executor/instruction/, server/service/grpc/
                    Does NOT touch: kv/ backend, keyspace, iterators, MVCC,
                                    durability, snapshots
```

**Shared touch point:** `encoding/encoding.rs` — Part 1 already moved
`rocks_configuration()` out of here. Part 2 doesn't touch this file. No conflict.

**Dependency:** `AttributeStorageStrategy` (Part 2) calls methods on whatever
storage backend is active (Part 1), but only through the existing
`ReadableSnapshot` / `WritableSnapshot` traits which both workstreams preserve.

**Recommended sequencing:** Either can go first. If working in parallel, the
only coordination needed is that both agree on the `KeyspaceSet` trait
(Part 1 defines it in `kv/keyspaces.rs`, Part 2 doesn't modify it).

---

# IMPLEMENTATION PRIORITY MATRIX

| Phase | Workstream | Status | Effort | Risk |
|-------|-----------|--------|--------|------|
| S0 | Storage: `kv/` crate with enum dispatch | **DONE** | — | — |
| S1 | Storage: in-memory backend (testing) | Pending | S | None |
| V1 | Value: descriptor + registry | Pending | S | None |
| S2 | Storage: redb backend | Pending | M | Medium |
| V2 | Value: AttributeID unification | Pending | M | Low |
| V3 | Value: expression op registry | Pending | M | Low |
| S3 | Storage: runtime backend selection | Pending | S | Low |
| V4 | Value: index strategy trait | Pending | L | Medium |
| V5 | Value: protocol + TypeQL ext | Pending | M | Medium |

S = small (1-3 days), M = medium (3-7 days), L = large (1-2 weeks)

---

# APPENDIX: FILE INVENTORY

## Files Changed for Storage Backend Abstraction (COMPLETED)

**New files (kv/ crate):**
- `kv/lib.rs` — `KVStore` enum dispatch
- `kv/iterator.rs` — `KVRangeIterator` enum dispatch
- `kv/write_batches.rs` — `KVWriteBatch` enum + `WriteBatches`
- `kv/keyspaces.rs` — `KeyspaceSet` trait, `Keyspaces`, `KeyspaceId`
- `kv/rocks/mod.rs` — `RocksKVStore` (extracted from `storage/keyspace/`)
- `kv/rocks/iterator.rs` — `RocksRangeIterator` (extracted)
- `kv/rocks/iterpool.rs` — Raw iterator pool (RocksDB-specific)
- `kv/rocks/pool.rs` — Connection pool

**Deleted files:**
- `storage/keyspace/keyspace.rs` (420 lines)
- `storage/keyspace/raw_iterator.rs` (158 lines)
- `storage/keyspace/mod.rs` (47 lines)
- `storage/keyspace/constants.rs` (12 lines)
- `storage/write_batches.rs` (83 lines)

**Moved files:**
- `storage/key_range.rs` → `common/primitive/key_range.rs`

**Modified files (73 total):**
- `storage/storage.rs` — uses `kv::Keyspaces` instead of internal keyspaces
- `storage/snapshot/snapshot.rs` — no longer generic over KV backend
- `storage/iterator.rs` — wraps `KVRangeIterator` instead of raw iterator
- `encoding/encoding.rs` — `EncodingKeyspace` implements `kv::KeyspaceSet`
- `database/database.rs` — opens via `KVStore::open_keyspaces()`
- Plus 60+ files across concept, compiler, function, ir, server for generic removal

**Unchanged:** All consumer code APIs (executor, concept, compiler, query, server)

## Files that Change for Value Type Extensibility

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
