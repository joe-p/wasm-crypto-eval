use sha2::{Digest, Sha256};
use std::sync::Mutex;
use wamr_runtime::{runtime::WamrRuntime, wamr_host_function};

/// Identity of a program, derived from the SHA-256 of its original module bytes.
///
/// Secrets are namespaced by this value, so a program can only ever reach the
/// secrets associated with its own code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ProgramId([u8; 32]);

/// Computes a program's identity from its module bytes.
///
/// Call this on the *original* bytes before [`Program::new`], which may mutate
/// the buffer in place.
///
/// [`Program::new`]: wamr_runtime::wamr_program::Program::new
pub fn program_id(program_bytes: &[u8]) -> ProgramId {
    let digest = Sha256::digest(program_bytes);
    ProgramId(digest.into())
}

pub trait SecretStore {
    fn read(program: &ProgramId, name: &str) -> Vec<u8>;
    fn write(program: &ProgramId, name: &str, data: &[u8]);
}

#[derive(Clone, Copy)]
struct StoreVtable {
    read: fn(&ProgramId, &str) -> Vec<u8>,
    write: fn(&ProgramId, &str, &[u8]),
}

// The host functions are capture-free `extern "C"` trampolines, so the store is
// selected by type at `create_runtime` time and reached through this global.
// Only one WamrRuntime may exist at a time, which makes the global unambiguous.
static SECRET_STORE: Mutex<Option<StoreVtable>> = Mutex::new(None);

/// Runs `f` with the configured store, returning `None` if no store is
/// configured or the lock is poisoned. Never panics.
fn with_store<R>(f: impl FnOnce(&StoreVtable) -> R) -> Option<R> {
    let guard = SECRET_STORE.lock().ok()?;
    guard.as_ref().map(f)
}

pub fn create_runtime<S: SecretStore>(store: S) -> WamrRuntime {
    let _ = store; // unused: the static trait is selected by type, not by value
    *SECRET_STORE.lock().expect("secret store mutex poisoned") = Some(StoreVtable {
        read: <S as SecretStore>::read,
        write: <S as SecretStore>::write,
    });

    #[wamr_host_function]
    fn store_read(ctx: Option<&ProgramId>, name: &[u8], out: &mut [u8]) -> u32 {
        // Identity comes from the host-set execution context, never from wasm.
        // Any missing context, malformed key, or store failure denies access.
        let Some(program) = ctx else {
            return 0;
        };
        let Ok(name) = std::str::from_utf8(name) else {
            return 0;
        };
        let Some(data) = with_store(|vtable| (vtable.read)(program, name)) else {
            return 0;
        };
        let len = data.len().min(out.len());
        out[..len].copy_from_slice(&data[..len]);
        len as u32
    }

    #[wamr_host_function]
    fn store_write(ctx: Option<&ProgramId>, name: &[u8], data: &[u8]) {
        let Some(program) = ctx else {
            return;
        };
        let Ok(name) = std::str::from_utf8(name) else {
            return;
        };
        with_store(|vtable| (vtable.write)(program, name, data));
    }

    WamrRuntime::new(
        vec![store_read_host_function(), store_write_host_function()],
        16 << 20,
    )
    .expect("init runtime")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::LazyLock;
    use std::sync::Mutex;
    use wamr_runtime::ERROR_BUFFER_SIZE;
    use wamr_runtime::wamr_program::{Program, ProgramConfig};

    // WAMR runtime creation is process-global, so tests that create a
    // WamrRuntime must not run concurrently.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct MemoryStore;

    type SecretsByProgram = HashMap<ProgramId, HashMap<String, Vec<u8>>>;

    static MEM_STORE: LazyLock<Mutex<SecretsByProgram>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    impl SecretStore for MemoryStore {
        fn read(program: &ProgramId, name: &str) -> Vec<u8> {
            MEM_STORE
                .lock()
                .unwrap()
                .get(program)
                .and_then(|secrets| secrets.get(name))
                .cloned()
                .unwrap_or_default()
        }

        fn write(program: &ProgramId, name: &str, data: &[u8]) {
            MEM_STORE
                .lock()
                .unwrap()
                .entry(*program)
                .or_default()
                .insert(name.to_string(), data.to_vec());
        }
    }

    fn program_config() -> ProgramConfig {
        ProgramConfig {
            error_buf: [0; ERROR_BUFFER_SIZE],
            stack_size: 1 << 16,
            app_heap_size: 1 << 16,
            max_pages: 0,
            instruction_count_limit: 100_000,
        }
    }

    /// Loads a module, binding its content-hash identity as host user data.
    fn load(wat: &str) -> Program {
        let mut program_bytes = wat::parse_str(wat).expect("parse WAT");
        let id = program_id(&program_bytes);
        let mut config = program_config();
        Program::new(&mut program_bytes, &mut config, Box::new(id)).expect("load module")
    }

    // Writes "value" to key "secret", then reads it back and returns the first
    // byte shifted up by the returned length. Requires an identity to succeed.
    const WRITER: &str = r#"(module
        (import "env" "store_write" (func $store_write (param i32 i32 i32 i32)))
        (import "env" "store_read" (func $store_read (param i32 i32 i32 i32) (result i32)))
        (import "env" "host_malloc" (func $host_malloc (param i64) (result i64)))
        (import "env" "host_free" (func $host_free (param i64)))
        (memory (export "memory") 1)
        (data (i32.const 0) "secret")
        (data (i32.const 16) "value")
        (func (export "program") (result i64)
            (local $out i64)
            (local $n i32)
            i32.const 0
            i32.const 6
            i32.const 16
            i32.const 5
            call $store_write

            i64.const 5
            call $host_malloc
            local.set $out

            i32.const 0
            i32.const 6
            local.get $out
            i32.wrap_i64
            i32.const 5
            call $store_read
            local.set $n

            local.get $out
            i32.wrap_i64
            i32.load8_u
            i64.extend_i32_u
            local.get $n
            i64.extend_i32_u
            i64.const 8
            i64.shl
            i64.or

            local.get $out
            call $host_free))"#;

    // Reads key "secret" into a fresh buffer and returns the number of bytes the
    // host reported.
    const READER: &str = r#"(module
        (import "env" "store_read" (func $store_read (param i32 i32 i32 i32) (result i32)))
        (import "env" "host_malloc" (func $host_malloc (param i64) (result i64)))
        (import "env" "host_free" (func $host_free (param i64)))
        (memory (export "memory") 1)
        (data (i32.const 0) "secret")
        (func (export "program") (result i64)
            (local $out i64)
            (local $n i32)
            i64.const 5
            call $host_malloc
            local.set $out

            i32.const 0
            i32.const 6
            local.get $out
            i32.wrap_i64
            i32.const 5
            call $store_read
            local.set $n

            local.get $out
            call $host_free

            local.get $n
            i64.extend_i32_u))"#;

    #[test]
    fn store_round_trips_through_wasm() {
        let _guard = TEST_LOCK.lock().unwrap();
        MEM_STORE.lock().unwrap().clear();

        let _runtime = create_runtime(MemoryStore);
        let program = load(WRITER);
        let result = program.call().expect("call program");

        // low byte: 'v' (118), high byte: length 5
        assert_eq!(result, (5u64 << 8) | u64::from(b'v'));
    }

    #[test]
    fn programs_cannot_read_each_others_secrets() {
        let _guard = TEST_LOCK.lock().unwrap();
        MEM_STORE.lock().unwrap().clear();

        let _runtime = create_runtime(MemoryStore);

        // The writer stores "value" under "secret" in its own namespace...
        let writer = load(WRITER);
        assert_eq!(
            writer.call().expect("call writer"),
            (5u64 << 8) | u64::from(b'v')
        );

        // ...but a different module (different bytes, different identity) sees
        // nothing. If its read leaked, it would report length 5.
        let reader = load(READER);
        assert_eq!(reader.call().expect("call reader"), 0);
    }

    #[test]
    fn host_call_without_matching_identity_is_denied() {
        let _guard = TEST_LOCK.lock().unwrap();
        MEM_STORE.lock().unwrap().clear();

        let _runtime = create_runtime(MemoryStore);

        // Seed a secret owned by the reader's identity, then load the same
        // module with a *different* user-data type: the accessor must not
        // downcast, so the read is denied even though the secret exists.
        let mut program_bytes = wat::parse_str(READER).expect("parse WAT");
        let owner = program_id(&program_bytes);
        MemoryStore::write(&owner, "secret", b"value");

        let mut config = program_config();
        let program =
            Program::new(&mut program_bytes, &mut config, Box::new(0u32)).expect("load module");
        assert_eq!(program.call().expect("call program"), 0);
    }
}
