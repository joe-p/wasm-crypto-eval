//! # SAFETY
//! This module simply wraps the unsafe WAMR bindings in safe Rust functions. The safety of
//! these functions is entirely dependent on WAMR C implementation and correct usage.
//!
//! If there is additional unsafe functionality that needs to be implemented for WAMR usage, it
//! SHOULD NOT be added here. This module is only for wrapping existing unsafe WAMR functions
//! without any changes to the signature or behavior.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

#[allow(warnings)]
pub mod wamr_c {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

pub fn wasm_memory_type_get_max_page_count(memory_type: wamr_c::wasm_memory_type_t) -> u32 {
    unsafe { wamr_c::wasm_memory_type_get_max_page_count(memory_type) }
}

pub fn wasm_runtime_get_export_type(
    module: wamr_c::wasm_module_t,
    export_index: i32,
    export_type: *mut wamr_c::wasm_export_t,
) {
    unsafe {
        wamr_c::wasm_runtime_get_export_type(module, export_index, export_type);
    }
}

pub fn wasm_runtime_get_export_count(module: wamr_c::wasm_module_t) -> i32 {
    unsafe { wamr_c::wasm_runtime_get_export_count(module) }
}

pub fn wasm_runtime_module_malloc(
    module_inst: wamr_c::wasm_module_inst_t,
    size: u64,
    p_native_addr: *mut *mut core::ffi::c_void,
) -> u64 {
    unsafe { wamr_c::wasm_runtime_module_malloc(module_inst, size, p_native_addr) }
}

pub fn wasm_runtime_module_free(module_inst: wamr_c::wasm_module_inst_t, ptr: u64) {
    unsafe {
        wamr_c::wasm_runtime_module_free(module_inst, ptr);
    }
}

pub fn wasm_runtime_destroy_exec_env(exec_env: *mut wamr_c::WASMExecEnv) {
    unsafe {
        wamr_c::wasm_runtime_destroy_exec_env(exec_env);
    }
}

pub fn wasm_runtime_deinstantiate(instance: *mut wamr_c::WASMModuleInstanceCommon) {
    unsafe {
        wamr_c::wasm_runtime_deinstantiate(instance);
    }
}

pub fn wasm_runtime_unload(module: *mut wamr_c::WASMModuleCommon) {
    unsafe {
        wamr_c::wasm_runtime_unload(module);
    }
}

pub fn wasm_runtime_load(
    buf: *mut u8,
    size: u32,
    error_buf: *mut core::ffi::c_char,
    error_buf_size: u32,
) -> wamr_c::wasm_module_t {
    unsafe { wamr_c::wasm_runtime_load(buf, size, error_buf, error_buf_size) }
}

pub fn wasm_runtime_instantiate_ex(
    module: wamr_c::wasm_module_t,
    args: *const wamr_c::InstantiationArgs,
    error_buf: *mut core::ffi::c_char,
    error_buf_size: u32,
) -> wamr_c::wasm_module_inst_t {
    unsafe { wamr_c::wasm_runtime_instantiate_ex(module, args, error_buf, error_buf_size) }
}

pub fn wasm_runtime_create_exec_env(
    instance: *mut wamr_c::WASMModuleInstanceCommon,
    stack_size: u32,
) -> *mut wamr_c::WASMExecEnv {
    unsafe { wamr_c::wasm_runtime_create_exec_env(instance, stack_size) }
}

pub fn wasm_runtime_lookup_function(
    instance: *mut wamr_c::WASMModuleInstanceCommon,
    name: *const core::ffi::c_char,
) -> wamr_c::wasm_function_inst_t {
    unsafe { wamr_c::wasm_runtime_lookup_function(instance, name) }
}

pub fn wasm_runtime_call_wasm_a(
    exec_env: *mut wamr_c::WASMExecEnv,
    func: wamr_c::wasm_function_inst_t,
    num_results: u32,
    results: *mut wamr_c::wasm_val_t,
    num_args: u32,
    args: *mut wamr_c::wasm_val_t,
) -> bool {
    unsafe {
        wamr_c::wasm_runtime_call_wasm_a(exec_env, func, num_results, results, num_args, args)
    }
}

pub fn wasm_runtime_get_exception(
    instance: *mut wamr_c::WASMModuleInstanceCommon,
) -> *const core::ffi::c_char {
    unsafe { wamr_c::wasm_runtime_get_exception(instance) }
}

pub fn wasm_runtime_destroy() {
    unsafe {
        wamr_c::wasm_runtime_destroy();
    }
}

pub fn wasm_runtime_full_init(args: *mut wamr_c::RuntimeInitArgs) -> bool {
    unsafe { wamr_c::wasm_runtime_full_init(args) }
}

pub fn wasm_runtime_register_natives(
    module_name: *const core::ffi::c_char,
    natives: *mut wamr_c::NativeSymbol,
    n_native_symbols: u32,
) -> bool {
    unsafe { wamr_c::wasm_runtime_register_natives(module_name, natives, n_native_symbols) }
}

pub fn wasm_runtime_get_user_data(
    exec_env: *mut wamr_c::WASMExecEnv,
) -> *mut core::ffi::c_void {
    unsafe { wamr_c::wasm_runtime_get_user_data(exec_env) }
}

pub fn wasm_runtime_set_user_data(
    exec_env: *mut wamr_c::WASMExecEnv,
    user_data: *mut core::ffi::c_void,
) {
    unsafe {
        wamr_c::wasm_runtime_set_user_data(exec_env, user_data);
    }
}

pub fn wasm_val_t_get_i64(v: &wamr_c::wasm_val_t) -> i64 {
    unsafe { v.of.i64_ }
}
