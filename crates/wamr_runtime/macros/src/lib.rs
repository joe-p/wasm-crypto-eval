//! Procedural macros for exposing Rust functions to WAMR as host functions.

use proc_macro::TokenStream;
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{parse_macro_input, spanned::Spanned, FnArg, ItemFn, ReturnType, Type};

/// Generates a [`WamrHostFunction`] for a Rust function.
///
/// The annotated function is left untouched and takes only the arguments that
/// the wasm module will provide; the WAMR execution environment is injected by
/// the generated trampoline and is not visible to the implementation:
///
/// ```ignore
/// #[wamr_host_function]
/// fn add(a: i64, b: i64) -> i64 {
///     a + b
/// }
/// ```
///
/// The macro emits an `extern "C"` trampoline that WAMR can call and a helper
/// named `<function>_host_function` returning the [`WamrHostFunction`]:
///
/// ```ignore
/// let host_functions = vec![add_host_function()];
/// let runtime = WamrRuntime::new(host_functions, heap_size)?;
/// ```
///
/// Supported parameter and return types:
/// - `i32`, `u32`, `i64`, `u64`, `f32`, `f64`
/// - `&[u8]` and `&mut [u8]` (encoded as WAMR `*~`, i.e. a bounds-checked
///   pointer plus length)
///
/// A parameter of type `Option<&T>` is treated as host user data rather than a
/// wasm argument: it is omitted from the WAMR signature and receives the value
/// installed with [`Program::new`] (as `Some(&T)`), or `None` if no matching
/// user data is present.
///
/// [`Program::new`]: wamr_runtime::wamr_program::Program::new
///
/// [`WamrHostFunction`]: wamr_runtime::runtime::WamrHostFunction
#[proc_macro_attribute]
pub fn wamr_host_function(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    match expand(func) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

enum ArgKind {
    Scalar {
        low: TokenStream2,
        wamr: Ident,
        signature: char,
    },
    Bytes {
        mutable: bool,
    },
    UserData {
        inner: Type,
    },
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
                    return Ok(ArgKind::Bytes {
                        mutable: reference.mutability.is_some(),
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

            let kind = match ident.as_str() {
                "i32" => (quote!(i32), format_ident!("I32"), 'i'),
                "u32" => (quote!(u32), format_ident!("I32"), 'i'),
                "i64" => (quote!(i64), format_ident!("I64"), 'I'),
                "u64" => (quote!(u64), format_ident!("I64"), 'I'),
                "f32" => (quote!(f32), format_ident!("F32"), 'f'),
                "f64" => (quote!(f64), format_ident!("F64"), 'F'),
                _ => {
                    return Err(syn::Error::new_spanned(
                        ty,
                        format!(
                            "unsupported WAMR host function type `{ident}`; \
                             supported types are i32, u32, i64, u64, f32, f64, &[u8], &mut [u8]"
                        ),
                    ));
                }
            };

            Ok(ArgKind::Scalar {
                low: kind.0,
                wamr: kind.1,
                signature: kind.2,
            })
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "unsupported WAMR host function parameter type",
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
            "`#[wamr_host_function]` does not support async functions",
        ));
    }
    if !func.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.generics,
            "`#[wamr_host_function]` does not support generic functions",
        ));
    }

    let mut trampoline_params = Vec::new();
    let mut call_args = Vec::new();
    let mut wamr_types = Vec::new();
    let mut signature = String::new();
    let mut ctx_bindings = Vec::new();

    for (index, input) in func.sig.inputs.iter().enumerate() {
        let FnArg::Typed(arg) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "`#[wamr_host_function]` does not support method receivers",
            ));
        };

        match classify(&arg.ty)? {
            ArgKind::Scalar {
                low,
                wamr,
                signature: sig,
            } => {
                let ident = format_ident!("__arg{}", index);
                trampoline_params.push(quote!(#ident: #low));
                call_args.push(quote!(#ident));
                wamr_types.push(quote!(::wamr_runtime::runtime::WamrType::#wamr));
                signature.push(sig);
            }
            ArgKind::Bytes { mutable } => {
                let ptr = format_ident!("__ptr{}", index);
                let len = format_ident!("__len{}", index);
                trampoline_params.push(quote!(#ptr: *mut u8));
                trampoline_params.push(quote!(#len: u32));

                let variant = if mutable {
                    format_ident!("MutByteSlice")
                } else {
                    format_ident!("ByteSlice")
                };
                wamr_types.push(quote!(::wamr_runtime::runtime::WamrType::#variant));
                signature.push_str("*~");

                let from_raw_parts = if mutable {
                    format_ident!("from_raw_parts_mut")
                } else {
                    format_ident!("from_raw_parts")
                };
                let data_type = if mutable {
                    quote!(&mut [u8])
                } else {
                    quote!(&[u8])
                };
                let value = quote! {
                    {
                        let __data: #data_type = unsafe {
                            ::core::slice::#from_raw_parts(
                                if #len == 0 {
                                    ::core::ptr::NonNull::<u8>::dangling().as_ptr()
                                } else {
                                    #ptr
                                },
                                #len as usize,
                            )
                        };
                        __data
                    }
                };
                call_args.push(value);
            }
            ArgKind::UserData { inner } => {
                let ident = format_ident!("__ctx{}", index);
                ctx_bindings.push(quote! {
                    let #ident =
                        ::wamr_runtime::runtime::user_data::<#inner>(__exec_env);
                });
                call_args.push(quote!(#ident.as_ref()));
            }
        }
    }

    let (ret_low, ret_wamr) = match &func.sig.output {
        ReturnType::Default => (None, None),
        ReturnType::Type(_, ty) if is_unit(ty) => (None, None),
        ReturnType::Type(_, ty) => match classify(ty)? {
            ArgKind::Scalar { low, wamr, .. } => (Some(low), Some(wamr)),
            ArgKind::Bytes { .. } => {
                return Err(syn::Error::new_spanned(
                    ty,
                    "byte slices cannot be returned from a WAMR host function",
                ));
            }
            ArgKind::UserData { .. } => {
                return Err(syn::Error::new_spanned(
                    ty,
                    "host user data cannot be returned from a WAMR host function",
                ));
            }
        },
    };

    let trampoline_ret = match &ret_low {
        Some(low) => quote!(-> #low),
        None => quote!(),
    };

    let ctor_args = if wamr_types.is_empty() {
        quote!(None)
    } else {
        quote!(Some(vec![#(#wamr_types),*]))
    };
    let ctor_ret = match &ret_wamr {
        Some(wamr) => quote!(Some(::wamr_runtime::runtime::WamrType::#wamr)),
        None => quote!(None),
    };

    let fn_name = &func.sig.ident;
    let trampoline = format_ident!("__wamr_host_{}", fn_name);
    let helper = format_ident!("{}_host_function", fn_name);
    let vis = &func.vis;

    Ok(quote! {
        #func

        #[doc(hidden)]
        #[allow(non_snake_case, dead_code, unused_variables)]
        extern "C" fn #trampoline(
            __exec_env: *mut ::core::ffi::c_void
            #(, #trampoline_params)*
        ) #trampoline_ret {
            #(#ctx_bindings)*
            #fn_name(#(#call_args),*)
        }

        #[allow(dead_code)]
        #vis fn #helper() -> ::wamr_runtime::runtime::WamrHostFunction {
            ::wamr_runtime::runtime::WamrHostFunction::new(
                stringify!(#fn_name).to_string(),
                #trampoline as *mut ::core::ffi::c_void,
                #ctor_args,
                #ctor_ret,
            )
        }
    })
}

fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}
