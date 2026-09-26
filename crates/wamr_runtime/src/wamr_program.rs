use crate::ERROR_BUFFER_SIZE;
use crate::error::{
    AotLengthTooLargeSnafu, AppHeapSizeTooLargeSnafu, CallWasmSnafu, CreateExecEnvSnafu,
    ErrorBufferSizeTooLargeSnafu, ErrorBufferTooSmallSnafu, InstantiateModuleSnafu, LoadModuleSnafu,
    MissingProgramFunctionSnafu, Result, WasmValueKindTooLargeSnafu,
};
use crate::runtime::{self, UserData};
use crate::wamr_sys;
use crate::wamr_sys::wamr_c;
use snafu::ensure;
use std::any::Any;
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
    // Owns the payload pointed to by this program's `exec_env` user data. The
    // heap allocation is stable across moves of `Program`, so the installed
    // pointer stays valid until `Drop` destroys the execution environment.
    user_data: Box<UserData>,
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
    pub fn new(
        program_bytes: &mut [u8],
        program_config: &mut ProgramConfig,
        user_data: Box<dyn Any + Send>,
    ) -> Result<Self> {
        let ProgramConfig {
            error_buf,
            stack_size,
            app_heap_size,
            max_pages,
            instruction_count_limit,
        } = program_config;

        ensure!(
            error_buf.len() >= ERROR_BUFFER_SIZE,
            ErrorBufferTooSmallSnafu {
                size: ERROR_BUFFER_SIZE
            }
        );
        error_buf.fill(0);

        let prog_len = u32::try_from(program_bytes.len())
            .map_err(|_| AotLengthTooLargeSnafu.build())?;
        let error_buf_size =
            u32::try_from(ERROR_BUFFER_SIZE).map_err(|_| ErrorBufferSizeTooLargeSnafu.build())?;
        let app_heap_size =
            u32::try_from(*app_heap_size).map_err(|_| AppHeapSizeTooLargeSnafu.build())?;

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
            return LoadModuleSnafu { message: err_msg }.fail();
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
            return InstantiateModuleSnafu { message: err_msg }.fail();
        }

        let exec_env = wamr_sys::wasm_runtime_create_exec_env(instance, *stack_size);
        if exec_env.is_null() {
            wamr_sys::wasm_runtime_deinstantiate(instance);
            wamr_sys::wasm_runtime_unload(module);
            return CreateExecEnvSnafu.fail();
        }

        unsafe {
            wamr_c::wasm_runtime_set_instruction_count_limit(exec_env, *instruction_count_limit);
        }

        let program_func = wamr_sys::wasm_runtime_lookup_function(instance, c"program".as_ptr());

        if program_func.is_null() {
            wamr_sys::wasm_runtime_destroy_exec_env(exec_env);
            wamr_sys::wasm_runtime_deinstantiate(instance);
            wamr_sys::wasm_runtime_unload(module);
            return MissingProgramFunctionSnafu.fail();
        }

        let program = Program {
            module,
            instance,
            exec_env,
            program_func,
            user_data: Box::new(UserData::new(user_data)),
        };

        runtime::install_user_data(program.exec_env, &program.user_data);

        Ok(program)
    }

    /// Borrows the user data this program was created with.
    pub fn user_data<T: Any>(&self) -> Option<&T> {
        self.user_data.value.as_ref().downcast_ref::<T>()
    }

    pub fn call(&self) -> Result<u64> {
        let kind = u8::try_from(wamr_c::wasm_valkind_enum_WASM_I64)
            .map_err(|_| WasmValueKindTooLargeSnafu.build())?;
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

            return CallWasmSnafu { message: msg }.fail();
        }

        Ok(wamr_sys::wasm_val_t_get_i64(&results[0]) as u64)
    }
}
