use crate::error::{
    FuelSnafu, InstantiateModuleSnafu, LoadModuleSnafu, MissingProgramFunctionSnafu,
    OutOfFuelSnafu, Result, TrapSnafu,
};
use crate::runtime::{HostState, WasmRuntime, store_limits};
use alloc::boxed::Box;
use alloc::string::ToString;
use core::any::Any;
use wasmi::{Instance, Module, Store, TrapCode, TypedFunc};

/// Per-program configuration.
pub struct ProgramConfig {
    /// Fuel granted for each call to the `program` export.
    pub fuel: u64,
    /// Cap on the program's linear memory, in bytes.
    pub memory_limit_bytes: usize,
}

impl Default for ProgramConfig {
    fn default() -> Self {
        ProgramConfig {
            fuel: 100_000,
            memory_limit_bytes: 16 << 20,
        }
    }
}

/// A loaded, instantiated WASM program.
pub struct Program {
    store: Store<HostState>,
    // Owns the module for as long as the instance is alive.
    _module: Module,
    // Kept alive alongside the typed function handle.
    _instance: Instance,
    program_func: TypedFunc<(), i64>,
    fuel: u64,
}

impl Program {
    /// Loads and instantiates a program.
    ///
    /// `user_data` is installed as the store's host state and is what host
    /// functions observe as their execution context.
    pub fn new(
        runtime: &WasmRuntime,
        program_bytes: &mut [u8],
        config: &ProgramConfig,
        user_data: Box<dyn Any + Send>,
    ) -> Result<Self> {
        let module = Module::new(runtime.engine(), &*program_bytes).map_err(|error| {
            LoadModuleSnafu {
                message: error.to_string(),
            }
            .build()
        })?;

        let mut store = Store::new(
            runtime.engine(),
            HostState::new(user_data, store_limits(config.memory_limit_bytes)),
        );
        store.limiter(|state| state.limits_mut());
        store.set_fuel(config.fuel).map_err(|error| {
            FuelSnafu {
                message: error.to_string(),
            }
            .build()
        })?;

        let instance = runtime
            .linker()
            .instantiate_and_start(&mut store, &module)
            .map_err(|error| {
                InstantiateModuleSnafu {
                    message: error.to_string(),
                }
                .build()
            })?;

        let program_func = instance
            .get_typed_func::<(), i64>(&store, "program")
            .map_err(|error| {
                MissingProgramFunctionSnafu {
                    message: error.to_string(),
                }
                .build()
            })?;

        Ok(Program {
            store,
            _module: module,
            _instance: instance,
            program_func,
            fuel: config.fuel,
        })
    }

    /// Borrows the user data this program was created with.
    pub fn user_data<T: Any>(&self) -> Option<&T> {
        self.store.data().value_ref::<T>()
    }

    /// Runs the `program` export to completion.
    ///
    /// Fuel is replenished before every call so independent invocations do not
    /// share one another's budget.
    pub fn call(&mut self) -> Result<u64> {
        self.store.set_fuel(self.fuel).map_err(|error| {
            FuelSnafu {
                message: error.to_string(),
            }
            .build()
        })?;

        match self.program_func.call(&mut self.store, ()) {
            Ok(value) => Ok(value as u64),
            Err(error) => {
                if error.as_trap_code() == Some(TrapCode::OutOfFuel) {
                    return OutOfFuelSnafu.fail();
                }
                TrapSnafu {
                    message: error.to_string(),
                }
                .fail()
            }
        }
    }
}
