extern crate self as wamr_runtime;

pub mod runtime;
pub mod wamr_program;
pub mod wamr_sys;

pub use wamr_runtime_macros::wamr_host_function;

pub const ERROR_BUFFER_SIZE: usize = 128;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wamr_program::ProgramConfig;
    use std::sync::Mutex;

    // WAMR runtime creation is process-global, so tests that create a
    // WamrRuntime must not run concurrently.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn wasm_program_returns_1337() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::runtime::WamrRuntime;
        use crate::wamr_program::{Program, ProgramConfig};

        // Minimal WASM module:
        //   (module (func (export "program") (result i64) i64.const 1337))
        const WAT: &str = "(module (func (export \"program\") (result i64) i64.const 1337))";

        let _runtime = WamrRuntime::new(vec![], 16 << 20).expect("init runtime");
        let mut program_bytes = wat::parse_str(WAT).expect("parse WAT");
        let mut config = ProgramConfig {
            error_buf: [0; ERROR_BUFFER_SIZE],
            stack_size: 1 << 16,
            app_heap_size: 1 << 16,
            max_pages: 0,
            instruction_count_limit: 100_000,
        };

        let program =
            Program::new(&mut program_bytes, &mut config, Box::new(())).expect("load module");
        assert_eq!(program.call().expect("call program"), 1337);
    }

    #[test]
    fn only_one_runtime_instance_allowed() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::runtime::WamrRuntime;

        let runtime = WamrRuntime::new(vec![], 16 << 20).expect("init runtime");

        let second = WamrRuntime::new(vec![], 16 << 20);
        assert!(
            second.is_err(),
            "second runtime instance should be rejected"
        );

        drop(runtime);

        let third = WamrRuntime::new(vec![], 16 << 20);
        assert!(
            third.is_ok(),
            "runtime should be creatable again after drop"
        );
    }

    #[test]
    fn host_function_macro_generates_signatures() {
        use crate::wamr_host_function;

        #[wamr_host_function]
        fn add(a: i64, b: i64) -> i64 {
            a + b
        }

        #[wamr_host_function]
        fn hash(input: &[u8], output: &mut [u8]) -> u32 {
            let _ = (input, output);
            0
        }

        #[wamr_host_function]
        fn noop() {}

        #[wamr_host_function]
        fn with_context(ctx: Option<&u32>, value: i64) -> i64 {
            let _ = ctx;
            value
        }

        assert_eq!(add_host_function().signature(), "(II)I");
        assert_eq!(hash_host_function().signature(), "(*~*~)i");
        assert_eq!(noop_host_function().signature(), "()");
        // The `Option<&T>` user-data parameter is not a wasm argument.
        assert_eq!(with_context_host_function().signature(), "(I)I");
    }

    #[test]
    fn host_function_macro_is_callable_from_wasm() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::runtime::WamrRuntime;
        use crate::wamr_host_function;
        use crate::wamr_program::{Program, ProgramConfig};

        #[wamr_host_function]
        fn add(a: i64, b: i64) -> i64 {
            a + b
        }

        const WAT: &str = r#"(module
            (import "env" "add" (func (param i64 i64) (result i64)))
            (func (export "program") (result i64)
                i64.const 3
                i64.const 4
                call 0))"#;

        let _runtime = WamrRuntime::new(vec![add_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = wat::parse_str(WAT).expect("parse WAT");
        let mut config = ProgramConfig {
            error_buf: [0; ERROR_BUFFER_SIZE],
            stack_size: 1 << 16,
            app_heap_size: 1 << 16,
            max_pages: 0,
            instruction_count_limit: 100_000,
        };

        let program =
            Program::new(&mut program_bytes, &mut config, Box::new(())).expect("load module");
        assert_eq!(program.call().expect("call program"), 7);
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

    #[test]
    fn host_function_slice_argument_receives_correct_length() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::runtime::WamrRuntime;
        use crate::wamr_host_function;
        use crate::wamr_program::Program;

        #[wamr_host_function]
        fn sum(data: &[u8]) -> i64 {
            data.iter().map(|byte| i64::from(*byte)).sum()
        }

        const WAT: &str = r#"(module
            (import "env" "sum" (func (param i32 i32) (result i64)))
            (memory (export "memory") 1)
            (data (i32.const 0) "hello")
            (func (export "program") (result i64)
                i32.const 0
                i32.const 5
                call 0))"#;

        let _runtime = WamrRuntime::new(vec![sum_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = wat::parse_str(WAT).expect("parse WAT");
        let mut config = program_config();

        let program =
            Program::new(&mut program_bytes, &mut config, Box::new(())).expect("load module");
        assert_eq!(program.call().expect("call program"), 532);
    }

    #[test]
    fn host_function_mutable_slice_argument_writes_through() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::runtime::WamrRuntime;
        use crate::wamr_host_function;
        use crate::wamr_program::Program;

        #[wamr_host_function]
        fn fill(data: &mut [u8], value: u32) {
            for byte in data.iter_mut() {
                *byte = value as u8;
            }
        }

        const WAT: &str = r#"(module
            (import "env" "fill" (func (param i32 i32 i32)))
            (memory (export "memory") 1)
            (func (export "program") (result i64)
                i32.const 0
                i32.const 4
                i32.const 42
                call 0
                i32.const 0
                i32.load8_u
                i64.extend_i32_u))"#;

        let _runtime =
            WamrRuntime::new(vec![fill_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = wat::parse_str(WAT).expect("parse WAT");
        let mut config = program_config();

        let program =
            Program::new(&mut program_bytes, &mut config, Box::new(())).expect("load module");
        assert_eq!(program.call().expect("call program"), 42);
    }

    #[test]
    fn host_function_receives_user_data() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::runtime::WamrRuntime;
        use crate::wamr_host_function;
        use crate::wamr_program::Program;

        #[wamr_host_function]
        fn get_identity(ctx: Option<&u32>, unused: i64) -> i64 {
            let _ = unused;
            i64::from(*ctx.unwrap_or(&0))
        }

        const WAT: &str = r#"(module
            (import "env" "get_identity" (func (param i64) (result i64)))
            (func (export "program") (result i64)
                i64.const 0
                call 0))"#;

        let _runtime =
            WamrRuntime::new(vec![get_identity_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = wat::parse_str(WAT).expect("parse WAT");
        let mut config = program_config();

        let program =
            Program::new(&mut program_bytes, &mut config, Box::new(7u32)).expect("load module");
        assert_eq!(program.call().expect("call program"), 7);
        // Host side can inspect the same payload.
        assert_eq!(program.user_data::<u32>(), Some(&7));
    }

    #[test]
    fn host_function_user_data_type_mismatch_is_none() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::runtime::WamrRuntime;
        use crate::wamr_host_function;
        use crate::wamr_program::Program;

        #[wamr_host_function]
        fn get_identity(ctx: Option<&u32>, unused: i64) -> i64 {
            let _ = unused;
            i64::from(*ctx.unwrap_or(&0))
        }

        const WAT: &str = r#"(module
            (import "env" "get_identity" (func (param i64) (result i64)))
            (func (export "program") (result i64)
                i64.const 0
                call 0))"#;

        let _runtime =
            WamrRuntime::new(vec![get_identity_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = wat::parse_str(WAT).expect("parse WAT");
        let mut config = program_config();

        // Installed payload is `u64`, but the host function asks for `u32`.
        // The checked downcast must yield `None` (deny) rather than reinterpret.
        let program =
            Program::new(&mut program_bytes, &mut config, Box::new(7u64)).expect("load module");
        assert_eq!(program.call().expect("call program"), 0);
        assert_eq!(program.user_data::<u32>(), None);
        assert_eq!(program.user_data::<u64>(), Some(&7));
    }
}
