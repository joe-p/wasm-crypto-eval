use crate::ERROR_BUFFER_SIZE;
use crate::wamr_sys;
use crate::wamr_sys::wamr_c;
use color_eyre::eyre::Result;
use color_eyre::eyre::{ensure, eyre};
use std::convert::TryFrom;
use std::ffi::c_char;

pub struct ProgramConfig {
    pub error_buf: [c_char; ERROR_BUFFER_SIZE],
    pub stack_size: u32,
    pub app_heap_size: usize,
    pub max_pages: u32,
    pub instruction_count_limit: i32,
}

pub struct Program {
    module: *mut wamr_c::WASMModuleCommon,

    instance: *mut wamr_c::WASMModuleInstanceCommon,
    exec_env: *mut wamr_c::WASMExecEnv,
    program_func: wamr_c::wasm_function_inst_t,
}

impl Drop for Program {
    fn drop(&mut self) {
        wamr_sys::wasm_runtime_destroy_exec_env(self.exec_env);
        wamr_sys::wasm_runtime_deinstantiate(self.instance);
        wamr_sys::wasm_runtime_unload(self.module);
    }
}

// Safety: Program can be sent between threads as long as long as it ultimately gets ran in its
// original runtime thread.
unsafe impl Send for Program {}

impl Program {
    pub fn new(program_bytes: &mut [u8], program_config: &mut ProgramConfig) -> Result<Self> {
        let ProgramConfig {
            error_buf,
            stack_size,
            app_heap_size,
            max_pages,
            instruction_count_limit,
        } = program_config;

        ensure!(
            error_buf.len() >= ERROR_BUFFER_SIZE,
            "Error buffer must be at least {ERROR_BUFFER_SIZE} bytes"
        );
        error_buf.fill(0);

        let prog_len =
            u32::try_from(program_bytes.len()).map_err(|_| eyre!("AOT length exceeds u32::MAX"))?;
        let error_buf_size = u32::try_from(ERROR_BUFFER_SIZE)
            .map_err(|_| eyre!("ERROR_BUFFER_SIZE exceeds u32::MAX"))?;
        let app_heap_size =
            u32::try_from(*app_heap_size).map_err(|_| eyre!("App heap size exceeds u32::MAX"))?;

        let module = wamr_sys::wasm_runtime_load(
            program_bytes.as_mut_ptr(),
            prog_len,
            error_buf.as_mut_ptr(),
            error_buf_size,
        );

        if module.is_null() {
            let err_msg = unsafe { std::ffi::CStr::from_ptr(error_buf.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            return Err(eyre!("Failed to load WASM module: {}", err_msg));
        }

        let export_count = wamr_sys::wasm_runtime_get_export_count(module);
        for export_index in 0..export_count {
            let mut export_type: wamr_c::wasm_export_t = Default::default();
            wamr_sys::wasm_runtime_get_export_type(module, export_index, &mut export_type);

            if export_type.kind == wamr_c::wasm_import_export_kind_t_WASM_IMPORT_EXPORT_KIND_MEMORY
            {
                *max_pages = core::cmp::min(
                    wamr_sys::wasm_memory_type_get_max_page_count(unsafe {
                        export_type.u.memory_type
                    }),
                    *max_pages,
                );
                break;
            }
        }

        let inst_args = wamr_c::InstantiationArgs {
            default_stack_size: *stack_size,
            host_managed_heap_size: app_heap_size,
            max_memory_pages: *max_pages,
        };

        let instance = wamr_sys::wasm_runtime_instantiate_ex(
            module,
            &inst_args,
            error_buf.as_mut_ptr(),
            error_buf_size,
        );

        if instance.is_null() {
            wamr_sys::wasm_runtime_unload(module);
            let err_msg = unsafe { std::ffi::CStr::from_ptr(error_buf.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            return Err(eyre!("Failed to instantiate WASM module: {}", err_msg));
        }

        let exec_env = wamr_sys::wasm_runtime_create_exec_env(instance, *stack_size);
        if exec_env.is_null() {
            wamr_sys::wasm_runtime_deinstantiate(instance);
            wamr_sys::wasm_runtime_unload(module);
            return Err(eyre!("Failed to create execution environment"));
        }

        unsafe {
            wamr_c::wasm_runtime_set_instruction_count_limit(exec_env, *instruction_count_limit);
        }

        let program_func = wamr_sys::wasm_runtime_lookup_function(instance, c"program".as_ptr());

        if program_func.is_null() {
            wamr_sys::wasm_runtime_destroy_exec_env(exec_env);
            wamr_sys::wasm_runtime_deinstantiate(instance);
            wamr_sys::wasm_runtime_unload(module);
            return Err(eyre!("Failed to find 'program' function"));
        }

        Ok(Program {
            module,
            instance,
            exec_env,
            program_func,
        })
    }

    pub fn call(&self) -> Result<u64> {
        let kind = u8::try_from(wamr_c::wasm_valkind_enum_WASM_I64)
            .map_err(|_| eyre!("WASM value kind does not fit in u8"))?;
        let mut results = [wamr_c::wasm_val_t {
            kind,
            of: wamr_c::wasm_val_t__bindgen_ty_1 { i64_: 0 },
            ..Default::default()
        }];

        if !wamr_sys::wasm_runtime_call_wasm_a(
            self.exec_env,
            self.program_func,
            1,
            results.as_mut_ptr(),
            0,
            std::ptr::null_mut(),
        ) {
            let ptr = wamr_sys::wasm_runtime_get_exception(self.instance);
            let msg = if ptr.is_null() {
                "unknown WAMR exception".to_string()
            } else {
                unsafe { std::ffi::CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .into_owned()
            };

            return Err(eyre!("WASM function call failed: {}", msg));
        }

        Ok(wamr_sys::wasm_val_t_get_i64(&results[0]) as u64)
    }
}
