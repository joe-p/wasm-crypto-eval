use alloc::string::String;
use snafu::Snafu;

/// Errors produced by the wasmi runtime wrapper.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("Failed to register host function `{name}`: {message}"))]
    RegisterHostFunction { name: String, message: String },

    #[snafu(display("Failed to configure store fuel: {message}"))]
    Fuel { message: String },

    #[snafu(display("Failed to load WASM module: {message}"))]
    LoadModule { message: String },

    #[snafu(display("Failed to instantiate WASM module: {message}"))]
    InstantiateModule { message: String },

    #[snafu(display("Failed to find `program` function: {message}"))]
    MissingProgramFunction { message: String },

    #[snafu(display("WASM execution ran out of fuel"))]
    OutOfFuel,

    #[snafu(display("WASM execution trapped: {message}"))]
    Trap { message: String },
}

pub type Result<T, E = Error> = core::result::Result<T, E>;
