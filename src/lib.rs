extern crate self as crypto_eval;

pub mod wamr_program;
pub mod wamr_runtime;
pub mod wamr_sys;

pub use crypto_eval_macros::wamr_host_function;

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
        use crate::wamr_program::{Program, ProgramConfig};
        use crate::wamr_runtime::WamrRuntime;

        // Minimal WASM module:
        //   (module (func (export "program") (result i64) i64.const 1337))
        const WASM: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic & version
            0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7e, // type: () -> i64
            0x03, 0x02, 0x01, 0x00, // func section: type 0
            0x07, 0x0b, 0x01, 0x07, b'p', b'r', b'o', b'g', b'r', b'a', b'm', 0x00,
            0x00, // export "program"
            0x0a, 0x07, 0x01, 0x05, 0x00, 0x42, 0xb9, 0x0a, 0x0b, // i64.const 1337; end
        ];

        let _runtime = WamrRuntime::new(vec![], 16 << 20).expect("init runtime");
        let mut program_bytes = WASM.to_vec();
        let mut config = ProgramConfig {
            error_buf: [0; ERROR_BUFFER_SIZE],
            stack_size: 1 << 16,
            app_heap_size: 1 << 16,
            max_pages: 0,
            instruction_count_limit: 100_000,
        };

        let program = Program::new(&mut program_bytes, &mut config).expect("load module");
        assert_eq!(program.call().expect("call program"), 1337);
    }

    #[test]
    fn only_one_runtime_instance_allowed() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::wamr_runtime::WamrRuntime;

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

        assert_eq!(add_host_function().signature(), "(II)I");
        assert_eq!(hash_host_function().signature(), "(*~*~)i");
        assert_eq!(noop_host_function().signature(), "()");
    }

    #[test]
    fn host_function_macro_is_callable_from_wasm() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::wamr_host_function;
        use crate::wamr_program::{Program, ProgramConfig};
        use crate::wamr_runtime::WamrRuntime;

        #[wamr_host_function]
        fn add(a: i64, b: i64) -> i64 {
            a + b
        }

        // (module
        //   (import "env" "add" (func (param i64 i64) (result i64)))
        //   (func (export "program") (result i64)
        //     i64.const 3
        //     i64.const 4
        //     call 0))
        const WASM: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic & version
            0x01, 0x0b, 0x02, 0x60, 0x02, 0x7e, 0x7e, 0x01, 0x7e, 0x60, 0x00, 0x01,
            0x7e, // type section
            0x02, 0x0b, 0x01, 0x03, b'e', b'n', b'v', 0x03, b'a', b'd', b'd', 0x00,
            0x00, // import section
            0x03, 0x02, 0x01, 0x01, // function section
            0x07, 0x0b, 0x01, 0x07, b'p', b'r', b'o', b'g', b'r', b'a', b'm', 0x00,
            0x01, // export section
            0x0a, 0x0a, 0x01, 0x08, 0x00, 0x42, 0x03, 0x42, 0x04, 0x10, 0x00,
            0x0b, // code section
        ];

        let _runtime = WamrRuntime::new(vec![add_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = WASM.to_vec();
        let mut config = ProgramConfig {
            error_buf: [0; ERROR_BUFFER_SIZE],
            stack_size: 1 << 16,
            app_heap_size: 1 << 16,
            max_pages: 0,
            instruction_count_limit: 100_000,
        };

        let program = Program::new(&mut program_bytes, &mut config).expect("load module");
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
        use crate::wamr_host_function;
        use crate::wamr_program::Program;
        use crate::wamr_runtime::WamrRuntime;

        #[wamr_host_function]
        fn sum(data: &[u8]) -> i64 {
            data.iter().map(|byte| i64::from(*byte)).sum()
        }

        // (module
        //   (memory (export "memory") 1)
        //   (data (i32.const 0) "hello")
        //   (import "env" "sum" (func (param i32 i32) (result i64)))
        //   (func (export "program") (result i64)
        //     i32.const 0
        //     i32.const 5
        //     call 0))
        const WASM: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic & version
            0x01, 0x0b, 0x02, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e, 0x60, 0x00, 0x01,
            0x7e, // type section
            0x02, 0x0b, 0x01, 0x03, b'e', b'n', b'v', 0x03, b's', b'u', b'm', 0x00,
            0x00, // import section
            0x03, 0x02, 0x01, 0x01, // function section
            0x05, 0x03, 0x01, 0x00, 0x01, // memory section
            0x07, 0x14, 0x02, 0x07, b'p', b'r', b'o', b'g', b'r', b'a', b'm', 0x00, 0x01, 0x06,
            b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00, // export section
            0x0a, 0x0a, 0x01, 0x08, 0x00, 0x41, 0x00, 0x41, 0x05, 0x10, 0x00,
            0x0b, // code section
            0x0b, 0x0b, 0x01, 0x00, 0x41, 0x00, 0x0b, 0x05, b'h', b'e', b'l', b'l',
            b'o', // data section
        ];

        let _runtime = WamrRuntime::new(vec![sum_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = WASM.to_vec();
        let mut config = program_config();

        let program = Program::new(&mut program_bytes, &mut config).expect("load module");
        assert_eq!(program.call().expect("call program"), 532);
    }

    #[test]
    fn host_function_mutable_slice_argument_writes_through() {
        let _guard = TEST_LOCK.lock().unwrap();
        use crate::wamr_host_function;
        use crate::wamr_program::Program;
        use crate::wamr_runtime::WamrRuntime;

        #[wamr_host_function]
        fn fill(data: &mut [u8], value: u32) {
            for byte in data.iter_mut() {
                *byte = value as u8;
            }
        }

        // (module
        //   (memory (export "memory") 1)
        //   (import "env" "fill" (func (param i32 i32 i32)))
        //   (func (export "program") (result i64)
        //     i32.const 0
        //     i32.const 4
        //     i32.const 42
        //     call 0
        //     i32.const 0
        //     i32.load8_u
        //     i64.extend_i32_u))
        const WASM: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic & version
            0x01, 0x0b, 0x02, 0x60, 0x03, 0x7f, 0x7f, 0x7f, 0x00, 0x60, 0x00, 0x01,
            0x7e, // type section
            0x02, 0x0c, 0x01, 0x03, b'e', b'n', b'v', 0x04, b'f', b'i', b'l', b'l', 0x00,
            0x00, // import section
            0x03, 0x02, 0x01, 0x01, // function section
            0x05, 0x03, 0x01, 0x00, 0x01, // memory section
            0x07, 0x14, 0x02, 0x07, b'p', b'r', b'o', b'g', b'r', b'a', b'm', 0x00, 0x01, 0x06,
            b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00, // export section
            0x0a, 0x12, 0x01, 0x10, 0x00, 0x41, 0x00, 0x41, 0x04, 0x41, 0x2a, 0x10, 0x00, 0x41,
            0x00, 0x2d, 0x00, 0x00, 0xad, 0x0b, // code section
        ];

        let _runtime =
            WamrRuntime::new(vec![fill_host_function()], 16 << 20).expect("init runtime");
        let mut program_bytes = WASM.to_vec();
        let mut config = program_config();

        let program = Program::new(&mut program_bytes, &mut config).expect("load module");
        assert_eq!(program.call().expect("call program"), 42);
    }
}
