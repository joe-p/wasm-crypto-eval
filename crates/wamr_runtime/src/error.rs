use alloc::string::String;
use snafu::Snafu;

/// Errors produced by the WAMR runtime wrapper.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("Host function name contains null byte: {name}"))]
    HostFunctionNameContainsNull { name: String },

    #[snafu(display("Host function signature contains null byte: {name}"))]
    HostFunctionSignatureContainsNull { name: String },

    #[snafu(display(
        "WAMR runtime already initialized; only one WamrRuntime instance is allowed"
    ))]
    RuntimeAlreadyInitialized,

    #[snafu(display("Failed to initialize WAMR runtime"))]
    RuntimeInit,

    #[snafu(display("Failed to register native symbols"))]
    RegisterNatives,

    #[snafu(display("Error buffer must be at least {size} bytes"))]
    ErrorBufferTooSmall { size: usize },

    #[snafu(display("AOT length exceeds u32::MAX"))]
    AotLengthTooLarge,

    #[snafu(display("ERROR_BUFFER_SIZE exceeds u32::MAX"))]
    ErrorBufferSizeTooLarge,

    #[snafu(display("App heap size exceeds u32::MAX"))]
    AppHeapSizeTooLarge,

    #[snafu(display("Failed to load WASM module: {message}"))]
    LoadModule { message: String },

    #[snafu(display("Failed to instantiate WASM module: {message}"))]
    InstantiateModule { message: String },

    #[snafu(display("Failed to create execution environment"))]
    CreateExecEnv,

    #[snafu(display("Failed to find 'program' function"))]
    MissingProgramFunction,

    #[snafu(display("WASM value kind does not fit in u8"))]
    WasmValueKindTooLarge,

    #[snafu(display("WASM function call failed: {message}"))]
    CallWasm { message: String },
}

pub type Result<T, E = Error> = core::result::Result<T, E>;
