//! Procedural macros for exposing Rust functions to wasmi as host functions.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{parse_macro_input, spanned::Spanned, FnArg, ItemFn, ReturnType, Type};

/// Generates a [`WasmHostFunction`] for a Rust function.
///
/// The annotated function is left untouched and takes only the arguments that
/// the wasm module will provide; the caller's execution state is injected by
/// the generated closure and is not visible to the implementation:
///
/// ```ignore
/// #[wasm_host_function]
/// fn add(a: i64, b: i64) -> i64 {
///     a + b
/// }
/// ```
///
/// The macro emits a helper named `<function>_host_function` returning the
/// [`WasmHostFunction`]:
///
/// ```ignore
/// let host_functions = vec![add_host_function()];
/// let runtime = WasmRuntime::new(&config, host_functions)?;
/// ```
///
/// Supported parameter and return types:
/// - `i32`, `u32`, `i64`, `u64`, `f32`, `f64`
/// - `&[u8]` and `&mut [u8]`, marshalled against the module's exported
///   `memory` (a bounds-checked pointer plus length)
///
/// A parameter of type `Option<&T>` is treated as host user data rather than a
/// wasm argument: it is omitted from the wasm signature and receives the value
/// installed with [`Program::new`] (as `Some(&T)`), or `None` if no matching
/// user data is present.
///
/// At most one `&mut [u8]` parameter is supported per function, and it may not
/// be combined with another `&mut [u8]`.
///
/// [`WasmHostFunction`]: wasmi_runtime::runtime::WasmHostFunction
/// [`Program::new`]: wasmi_runtime::program::Program::new
#[proc_macro_attribute]
pub fn wasm_host_function(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    match expand(func) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

enum ArgKind {
    Scalar { low: TokenStream2 },
    ImmutableBytes,
    MutableBytes,
    UserData { inner: Type },
}

/// Returns the `T` in an `Option<T>` path type, if the type is one.
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    match &args.args[0] {
        syn::GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn classify(ty: &Type) -> syn::Result<ArgKind> {
    if let Some(inner) = option_inner(ty) {
        return match inner {
            Type::Reference(reference)
                if reference.mutability.is_none()
                    && !matches!(&*reference.elem, Type::Slice(_)) =>
            {
                Ok(ArgKind::UserData {
                    inner: (*reference.elem).clone(),
                })
            }
            _ => Err(syn::Error::new_spanned(
                ty,
                "unsupported `Option<..>` parameter; expected `Option<&T>` for host user data",
            )),
        };
    }

    match ty {
        Type::Reference(reference) => {
            if let Type::Slice(slice) = &*reference.elem {
                if is_type(&slice.elem, "u8") {
                    return Ok(if reference.mutability.is_some() {
                        ArgKind::MutableBytes
                    } else {
                        ArgKind::ImmutableBytes
                    });
                }
                return Err(syn::Error::new_spanned(
                    &slice.elem,
                    "only `&[u8]` and `&mut [u8]` slice parameters are supported",
                ));
            }
            Err(syn::Error::new_spanned(
                ty,
                "unsupported reference parameter type; expected `&[u8]` or `&mut [u8]`",
            ))
        }
        Type::Path(path) => {
            let ident = path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default();

            let low = match ident.as_str() {
                "i32" => quote!(i32),
                "u32" => quote!(u32),
                "i64" => quote!(i64),
                "u64" => quote!(u64),
                "f32" => quote!(f32),
                "f64" => quote!(f64),
                _ => {
                    return Err(syn::Error::new_spanned(
                        ty,
                        format!(
                            "unsupported host function type `{ident}`; \
                             supported types are i32, u32, i64, u64, f32, f64, &[u8], &mut [u8]"
                        ),
                    ));
                }
            };

            Ok(ArgKind::Scalar { low })
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "unsupported host function parameter type",
        )),
    }
}

fn is_type(ty: &Type, name: &str) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == name),
        _ => false,
    }
}

fn expand(func: ItemFn) -> syn::Result<TokenStream2> {
    if let Some(asyncness) = func.sig.asyncness {
        return Err(syn::Error::new(
            asyncness.span(),
            "`#[wasm_host_function]` does not support async functions",
        ));
    }
    if !func.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.generics,
            "`#[wasm_host_function]` does not support generic functions",
        ));
    }

    let mut closure_params = Vec::new();
    let mut call_args = Vec::new();
    let mut ctx_stmts = Vec::new();
    let mut immutable_stmts = Vec::new();
    let mut mutable_stmts = Vec::new();

    for (index, input) in func.sig.inputs.iter().enumerate() {
        let FnArg::Typed(arg) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "`#[wasm_host_function]` does not support method receivers",
            ));
        };

        match classify(&arg.ty)? {
            ArgKind::Scalar { low, .. } => {
                let ident = format_ident!("__arg{}", index);
                closure_params.push(quote!(#ident: #low));
                call_args.push(quote!(#ident));
            }
            ArgKind::ImmutableBytes => {
                let ptr = format_ident!("__ptr{}", index);
                let len = format_ident!("__len{}", index);
                let data = format_ident!("__data{}", index);
                closure_params.push(quote!(#ptr: i32));
                closure_params.push(quote!(#len: i32));
                immutable_stmts.push(quote! {
                    let #data: ::wasmi_runtime::__private::Vec<u8> = {
                        match ::wasmi_runtime::wasmi::Caller::get_export(&__caller, "memory") {
                            ::core::option::Option::Some(
                                ::wasmi_runtime::wasmi::Extern::Memory(__mem),
                            ) => {
                                let __start = #ptr as usize;
                                let __end = __start.saturating_add(#len as usize);
                                if __end <= ::wasmi_runtime::wasmi::Memory::data_size(&__mem, &__caller) {
                                    let mut __buf =
                                        ::wasmi_runtime::__private::vec![0u8; #len as usize];
                                    ::wasmi_runtime::wasmi::Memory::read(
                                        &__mem,
                                        &__caller,
                                        __start,
                                        &mut __buf,
                                    )
                                    .ok();
                                    __buf
                                } else {
                                    ::wasmi_runtime::__private::Vec::new()
                                }
                            }
                            _ => ::wasmi_runtime::__private::Vec::new(),
                        }
                    };
                });
                call_args.push(quote!(#data.as_slice()));
            }
            ArgKind::MutableBytes => {
                let ptr = format_ident!("__ptr{}", index);
                let len = format_ident!("__len{}", index);
                let data = format_ident!("__data{}", index);
                let empty = format_ident!("__empty{}", index);
                closure_params.push(quote!(#ptr: i32));
                closure_params.push(quote!(#len: i32));
                mutable_stmts.push(quote! {
                    let mut #empty: ::wasmi_runtime::__private::Vec<u8> =
                        ::wasmi_runtime::__private::Vec::new();
                    let #data: &mut [u8] = {
                        match ::wasmi_runtime::wasmi::Caller::get_export(&__caller, "memory") {
                            ::core::option::Option::Some(
                                ::wasmi_runtime::wasmi::Extern::Memory(__mem),
                            ) => {
                                let __start = #ptr as usize;
                                let __end = __start.saturating_add(#len as usize);
                                if __end <= ::wasmi_runtime::wasmi::Memory::data_size(&__mem, &__caller) {
                                    let __bytes =
                                        ::wasmi_runtime::wasmi::Memory::data_mut(&__mem, &mut __caller);
                                    &mut __bytes[__start..__end]
                                } else {
                                    #empty.as_mut_slice()
                                }
                            }
                            _ => #empty.as_mut_slice(),
                        }
                    };
                });
                call_args.push(quote!(#data));
            }
            ArgKind::UserData { inner } => {
                let ident = format_ident!("__ctx{}", index);
                ctx_stmts.push(quote! {
                    let #ident = ::wasmi_runtime::runtime::HostState::get::<#inner>(
                        ::wasmi_runtime::wasmi::Caller::data(&__caller),
                    );
                });
                call_args.push(quote!(#ident.as_ref()));
            }
        }
    }

    let ret_low = match &func.sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) if is_unit(ty) => None,
        ReturnType::Type(_, ty) => match classify(ty)? {
            ArgKind::Scalar { low, .. } => Some(low),
            ArgKind::ImmutableBytes | ArgKind::MutableBytes => {
                return Err(syn::Error::new_spanned(
                    ty,
                    "byte slices cannot be returned from a host function",
                ));
            }
            ArgKind::UserData { .. } => {
                return Err(syn::Error::new_spanned(
                    ty,
                    "host user data cannot be returned from a host function",
                ));
            }
        },
    };

    let ret_ann = match &ret_low {
        Some(low) => quote!(-> #low),
        None => quote!(),
    };

    let fn_name = &func.sig.ident;
    let helper = format_ident!("{}_host_function", fn_name);
    let vis = &func.vis;

    Ok(quote! {
        #func

        #[allow(non_snake_case, dead_code, unused_variables, unused_mut, clippy::needless_borrow)]
        #vis fn #helper() -> ::wasmi_runtime::runtime::WasmHostFunction {
            ::wasmi_runtime::runtime::WasmHostFunction::new(
                ::wasmi_runtime::__private::ToString::to_string(&stringify!(#fn_name)),
                |__linker: &mut ::wasmi_runtime::wasmi::Linker<
                    ::wasmi_runtime::runtime::HostState,
                >| {
                    ::wasmi_runtime::wasmi::Linker::func_wrap(
                        __linker,
                        "env",
                        stringify!(#fn_name),
                        |__caller: ::wasmi_runtime::wasmi::Caller<
                            '_,
                            ::wasmi_runtime::runtime::HostState,
                        > #(, #closure_params)*| #ret_ann {
                            #[allow(unused_mut)]
                            let mut __caller = __caller;
                            #(#ctx_stmts)*
                            #(#immutable_stmts)*
                            #(#mutable_stmts)*
                            #fn_name(#(#call_args),*)
                        },
                    )
                    .map(|_| ())
                },
            )
        }
    })
}

fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}
