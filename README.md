# wasm-crypto-eval

A lightweight, `no_std`-compatible WebAssembly execution sandbox with cryptographic module identity, namespaced secret storage, and deterministic resource metering built on top of [Wasmi](https://github.com/wasmi-labs/wasmi).

## Overview

`wasm-crypto-eval` provides a secure, sandboxed execution environment where WebAssembly guest programs are cryptographically isolated. A program's identity is derived directly from the content hash (SHA-256) of its original module bytecode. Per-program secrets and state are strictly partitioned by this identity, ensuring that guest programs can never forge credentials or access data belonging to other modules.

### Key Features

- **Content-Addressed Identity (`ProgramId`)**: Modules are assigned a SHA-256 identity based on their immutable bytecode. Identity is injected strictly via the host execution context, making it impossible for guest code to tamper with or spoof.
- **Namespaced Secret Storage (`SecretStore`)**: Host-provided key-value storage for reading and writing secrets, strictly namespaced by `ProgramId`.
- **Resource Metering & Sandboxing**:
  - **Fuel Metering**: Configurable per-call fuel limits prevent denial-of-service (e.g., infinite loops). Fuel is replenished on every call.
  - **Memory Limits**: Linear memory is capped via store limiters to prevent out-of-memory traps.
  - **Stack & Call Recursion Controls**: Engine-level limits on maximum stack height and recursion depth.
- **Ergonomic Host Functions (`#[wasm_host_function]`)**: A procedural macro that automatically marshals scalar values, validates and copies guest linear memory slices (`&[u8]`, `&mut [u8]`), and safely downcasts host user data (`Option<&T>`).
- **`no_std` Support**: Core crates (`wasmi_runtime` and `wasm-crypto-eval`) support `#![no_std]` (with `extern crate alloc`), allowing execution in resource-constrained, embedded, or enclave environments.

---

## Workspace Structure

The project is structured as a Cargo workspace with three crates:

| Crate | Path | Description |
|---|---|---|
| [`wasm-crypto-eval`](crates/wasm-crypto-eval) | `crates/wasm-crypto-eval` | Cryptographic program identification (`ProgramId`), `SecretStore` trait, and runtime integration with `store_read` and `store_write` host functions. |
| [`wasmi_runtime`](crates/wasmi_runtime) | `crates/wasmi_runtime` | Safe `no_std` wrapper around Wasmi v2 providing engine configuration, instance lifecycles, fuel/memory metering, and typed function execution. |
| [`wasmi_runtime-macros`](crates/wasmi_runtime/macros) | `crates/wasmi_runtime/macros` | Procedural macro (`#[wasm_host_function]`) for declaring type-safe host functions with automated memory marshalling. |

---

## Security & Architecture Model

```
+-------------------------------------------------------------+
|                     Host Environment                        |
|                                                             |
|   Module Bytecode ---> SHA-256 Digest ---> ProgramId        |
|                                                |            |
|                                      (injected via HostState)
|                                                v            |
|   +-----------------------------------------------------+   |
|   |                   Wasmi Sandbox                     |   |
|   |                                                     |   |
|   |   +---------------------------------------------+   |   |
|   |   |                Wasm Program                 |   |   |
|   |   |  - Linear Memory (capped)                   |   |   |
|   |   |  - Fuel Metered Execution                   |   |   |
|   |   +---------------------------------------------+   |   |
|   |          |                               ^          |   |
|   |          | call env.store_read(key, buf) |          |   |
|   |          v                               |          |   |
|   +----------|-------------------------------|----------+   |
|              |                               |              |
|              v                               |              |
|   #[wasm_host_function]                      |              |
|   store_read(ctx: Option<&ProgramId>, ...)   |              |
|              |                               |              |
|              v                               |              |
|       SecretStore::read(ProgramId, key) -----+              |
|                                                             |
+-------------------------------------------------------------+
```

1. **Bytecode Identity**: Before instantiation, `program_id(&program_bytes)` computes the SHA-256 hash of the Wasm module bytes.
2. **Context Injection**: The resulting `ProgramId` is boxed and passed to `Program::new` as host user data inside `HostState`.
3. **Execution & Access**: When the Wasm module calls `store_read` or `store_write`, the `#[wasm_host_function]` macro extracts the `Option<&ProgramId>` reference from the Wasmi store. If the caller lacks a valid `ProgramId`, or if downcasting fails, access is denied (returning 0 or early exiting).
4. **Zero Cross-Talk**: Because keys are partitioned by `ProgramId`, two different programs querying the same secret key (e.g., `"api_key"`) will only ever observe their own distinct values.

---

## Usage

### 1. Defining and Initializing a Secret Store

Implement the `SecretStore` trait for your backend (e.g., in-memory map, database, or hardware secure enclave):

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use wasm_crypto_eval::{create_runtime, program_id, ProgramId, SecretStore};
use wasmi_runtime::program::{Program, ProgramConfig};

struct InMemorySecretStore {
    secrets: Mutex<HashMap<ProgramId, HashMap<String, Vec<u8>>>>,
}

impl SecretStore for InMemorySecretStore {
    fn read(&self, program: &ProgramId, name: &str) -> Vec<u8> {
        self.secrets
            .lock()
            .unwrap()
            .get(program)
            .and_then(|map| map.get(name))
            .cloned()
            .unwrap_or_default()
    }

    fn write(&self, program: &ProgramId, name: &str, data: &[u8]) {
        self.secrets
            .lock()
            .unwrap()
            .entry(*program)
            .or_default()
            .insert(name.to_string(), data.to_vec());
    }
}
```

### 2. Loading and Running a Program

```rust
let store = InMemorySecretStore {
    secrets: Mutex::new(HashMap::new()),
};

// Create the runtime with store_read and store_write host functions linked
let runtime = create_runtime(store);

// WASM bytecode (or compiled WAT)
let mut wasm_bytes = wat::parse_str(r#"
    (module
        (import "env" "store_read" (func $store_read (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "secret_key")
        (func (export "program") (result i64)
            ;; Read secret into offset 32, max 16 bytes
            i32.const 0   ;; key ptr
            i32.const 10  ;; key len
            i32.const 32  ;; out ptr
            i32.const 16  ;; out len
            call $store_read
            i64.extend_i32_u)
    )
"#).unwrap();

// Derive program identity from original bytecode
let id = program_id(&wasm_bytes);

let config = ProgramConfig {
    fuel: 100_000,
    memory_limit_bytes: 64 * 1024,
};

let mut program = Program::new(
    &runtime,
    &mut wasm_bytes,
    &config,
    Box::new(id),
).expect("failed to load program");

let result = program.call().expect("execution failed");
```

### 3. Writing Custom Host Functions

The `#[wasm_host_function]` attribute simplifies exposing Rust functions to WebAssembly:

```rust
use wasmi_runtime::wasm_host_function;

// Scalar arguments & return values
#[wasm_host_function]
fn add(a: i64, b: i64) -> i64 {
    a + b
}

// Memory slices (automatically checked against linear memory bounds)
#[wasm_host_function]
fn hash_data(data: &[u8], out: &mut [u8]) -> u32 {
    let len = data.len().min(out.len());
    out[..len].copy_from_slice(&data[..len]);
    len as u32
}

// Injected host user data (omitted from the Wasm-facing signature)
#[wasm_host_function]
fn get_context_id(ctx: Option<&u32>) -> u32 {
    *ctx.unwrap_or(&0)
}
```

Register the generated helper when creating the runtime:

```rust
use wasmi_runtime::runtime::{RuntimeConfig, WasmRuntime};

let runtime = WasmRuntime::new(
    &RuntimeConfig::default(),
    vec![add_host_function(), hash_data_host_function()],
)?;
```

---

## Building and Testing

### Prerequisites

- Rust 2024 edition (Rust 1.85+ recommended)

### Build

```bash
cargo build --workspace
```

### Run Tests

Run all unit and integration tests across the workspace:

```bash
cargo test
```

### Linting

```bash
cargo clippy --all-targets
```

---

## License

Distributed under the MIT or Apache-2.0 license.

