use crate::wamr_sys::{self, wamr_c};
use color_eyre::eyre::{Context, Result, eyre};
use std::any::Any;
use std::ffi::c_void;
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};

// Ensures that only one WamrRuntime instance exists at a time. WAMR's own
// init/full_init is ref-counted and silently ignores init arguments on
// subsequent calls, so this guard lives in the Rust wrapper.
static RUNTIME_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Type-erased payload attached to a program's execution environment.
///
/// WAMR's `exec_env->user_data` is a single opaque pointer. `UserData` is the
/// stable, thin allocation that this crate installs there; keeping the payload
/// behind a [`Box<dyn Any>`] lets [`user_data`] recover it with a checked
/// downcast instead of an unchecked pointer cast.
pub(crate) struct UserData {
    pub(crate) value: Box<dyn Any + Send>,
}

impl UserData {
    pub(crate) fn new(value: Box<dyn Any + Send>) -> Self {
        UserData { value }
    }
}

/// Installs `user_data` into the execution environment.
///
/// Crate-private because the soundness of [`user_data`] relies on this crate
/// being the only writer of `exec_env->user_data`.
pub(crate) fn install_user_data(exec_env: *mut wamr_c::WASMExecEnv, user_data: &UserData) {
    wamr_sys::wasm_runtime_set_user_data(exec_env, user_data as *const UserData as *mut c_void);
}

/// Retrieves the user data installed for the calling program.
///
/// Returns an owned clone rather than a borrow, so callers can never hold a
/// reference past the lifetime of the execution environment. A mismatched `T`
/// (or a missing payload) yields `None` instead of undefined behavior.
pub fn user_data<T: Any + Clone>(exec_env: *mut c_void) -> Option<T> {
    let header = wamr_sys::wasm_runtime_get_user_data(exec_env as *mut wamr_c::WASMExecEnv)
        as *const UserData;
    if header.is_null() {
        return None;
    }

    // SAFETY: `exec_env` is a WAMR execution environment whose user data is only
    // ever installed by `install_user_data`, so it is either null (checked above)
    // or a valid `UserData` allocated by `Program`.
    let header = unsafe { &*header };
    (header.value.as_ref() as &dyn Any)
        .downcast_ref::<T>()
        .cloned()
}

pub struct WamrRuntime {
    heap: Vec<u8>,
    native_symbols: Vec<wamr_c::NativeSymbol>,
    // A Vec to hold CString references to ensure they live as long as the runtime
    // This seems preferable to having to deal with lifetimes since the allocations
    // only happen once at runtime initialization
    _c_strings: Vec<std::ffi::CString>,
}

impl Drop for WamrRuntime {
    fn drop(&mut self) {
        wamr_sys::wasm_runtime_destroy();
        RUNTIME_INITIALIZED.store(false, Ordering::Release);
    }
}

extern "C" fn host_malloc(exec_env: *mut c_void, size: u64) -> u64 {
    let module_inst =
        unsafe { wamr_c::wasm_runtime_get_module_inst(exec_env as *mut wamr_c::WASMExecEnv) };

    wamr_sys::wasm_runtime_module_malloc(module_inst, size, std::ptr::null_mut())
}

extern "C" fn host_free(exec_env: *mut c_void, ptr: u64) {
    let module_inst =
        unsafe { wamr_c::wasm_runtime_get_module_inst(exec_env as *mut wamr_c::WASMExecEnv) };

    wamr_sys::wasm_runtime_module_free(module_inst, ptr);
}

#[derive(Clone)]
pub enum WamrType {
    I64,
    I32,
    F32,
    F64,
    ByteSlice,
    MutByteSlice,
}

impl Display for WamrType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WamrType::I64 => write!(f, "I"),
            WamrType::I32 => write!(f, "i"),
            WamrType::F32 => write!(f, "f"),
            WamrType::F64 => write!(f, "F"),
            WamrType::ByteSlice => write!(f, "*~"),
            WamrType::MutByteSlice => write!(f, "*~"),
        }
    }
}

#[derive(Clone)]
pub struct WamrHostFunction {
    name: String,
    function: *mut c_void,
    args: Option<Vec<WamrType>>,
    return_type: Option<WamrType>,
}

// Safety: WamrHostFunction is not Send out of the box because it contains a raw pointer.
// This pointer, however, is expected to be a pointer to a function which is safe to call from multiple threads.
unsafe impl Send for WamrHostFunction {}

impl WamrHostFunction {
    pub fn new(
        name: String,
        function: *mut c_void,
        args: Option<Vec<WamrType>>,
        return_type: Option<WamrType>,
    ) -> Self {
        WamrHostFunction {
            name: name.to_string(),
            function,
            args,
            return_type,
        }
    }

    pub fn signature(&self) -> String {
        let mut signature = String::new();
        signature.push('(');
        if let Some(args) = &self.args {
            for arg in args {
                signature.push_str(&arg.to_string());
            }
        }
        signature.push(')');
        if let Some(ret_type) = &self.return_type {
            signature.push_str(&ret_type.to_string());
        }
        signature
    }
}

impl WamrRuntime {
    pub fn new(host_functions: Vec<WamrHostFunction>, runtime_heap_size: usize) -> Result<Self> {
        let mut c_strings: Vec<std::ffi::CString> = vec![];
        let mut native_symbols: Vec<wamr_c::NativeSymbol> =
            Vec::with_capacity(host_functions.len() + 1);

        for host_fn in &host_functions {
            let name = std::ffi::CString::new(host_fn.name.clone()).wrap_err_with(|| {
                format!("Host function name contains null byte: {}", host_fn.name)
            })?;
            let signature = std::ffi::CString::new(host_fn.signature()).wrap_err_with(|| {
                format!(
                    "Host function signature contains null byte: {}",
                    host_fn.name
                )
            })?;

            c_strings.push(name);
            c_strings.push(signature);

            let symbol = c_strings[c_strings.len() - 2].as_ptr();
            let signature_ptr = c_strings[c_strings.len() - 1].as_ptr();

            native_symbols.push(wamr_c::NativeSymbol {
                symbol,
                func_ptr: host_fn.function,
                signature: signature_ptr,
                ..Default::default()
            });
        }

        native_symbols.push(wamr_c::NativeSymbol {
            symbol: c"host_malloc".as_ptr(),
            func_ptr: host_malloc as *mut c_void,
            signature: c"(I)I".as_ptr(),
            ..Default::default()
        });

        native_symbols.push(wamr_c::NativeSymbol {
            symbol: c"host_free".as_ptr(),
            func_ptr: host_free as *mut c_void,
            signature: c"(I)".as_ptr(),
            ..Default::default()
        });

        if RUNTIME_INITIALIZED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(eyre!(
                "WAMR runtime already initialized; only one WamrRuntime instance is allowed"
            ));
        }

        let runtime = WamrRuntime {
            native_symbols,
            heap: vec![0u8; runtime_heap_size],
            _c_strings: c_strings,
        };

        let mut init_args = wamr_c::RuntimeInitArgs {
            mem_alloc_type: wamr_c::mem_alloc_type_t_Alloc_With_Pool,
            running_mode: wamr_c::RunningMode_Mode_Interp,
            mem_alloc_option: wamr_c::MemAllocOption {
                pool: wamr_c::MemAllocOption__bindgen_ty_1 {
                    heap_buf: runtime.heap.as_ptr() as *mut c_void,
                    heap_size: runtime_heap_size as u32,
                },
            },
            ..Default::default()
        };

        if !wamr_sys::wasm_runtime_full_init(&mut init_args as *mut wamr_c::RuntimeInitArgs) {
            return Err(eyre!("Failed to initialize WAMR runtime"));
        }

        if !wamr_sys::wasm_runtime_register_natives(
            c"env".as_ptr(),
            runtime.native_symbols.as_ptr() as *mut wamr_c::NativeSymbol,
            runtime.native_symbols.len() as u32,
        ) {
            return Err(eyre!("Failed to register native symbols"));
        }

        Ok(runtime)
    }
}
