pub mod wamr_program;
pub mod wamr_runtime;
pub mod wamr_sys;

pub const ERROR_BUFFER_SIZE: usize = 128;

#[cfg(test)]
mod tests {
    use super::*;
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
}
