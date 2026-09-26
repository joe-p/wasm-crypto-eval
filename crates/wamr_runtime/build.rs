fn repo_root() -> std::path::PathBuf {
    std::fs::canonicalize(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("Failed to canonicalize repo root path")
}

fn wamr_root() -> String {
    repo_root()
        .join("wasm-micro-runtime")
        .to_str()
        .unwrap()
        .to_string()
}

fn add_include_path(builder: bindgen::Builder, path: &str) -> bindgen::Builder {
    builder.clang_arg(format!("-I{}/{}", wamr_root(), path))
}

fn main() {
    println!("cargo:rerun-if-changed={}/core", wamr_root());

    let mut builder = bindgen::Builder::default()
        .header(format!("{}/core/iwasm/include/wasm_export.h", wamr_root()))
        .header(format!("{}/core/iwasm/include/aot_export.h", wamr_root()));

    builder = add_include_path(builder, "core/iwasm/include");
    builder = add_include_path(builder, "core/iwasm/interpreter");
    builder = add_include_path(builder, "core/iwasm/aot");
    builder = add_include_path(builder, "core/iwasm/libraries/libc-builtin");
    builder = add_include_path(builder, "core/iwasm/common");
    builder = add_include_path(builder, "core/shared/include");
    builder = add_include_path(builder, "core/shared/platform/include");
    builder = add_include_path(builder, "core/shared/platform/linux");
    builder = add_include_path(builder, "core/shared/platform/common/posix");
    builder = add_include_path(builder, "core/shared/mem-alloc");
    builder = add_include_path(builder, "core/shared/utils");
    builder = add_include_path(builder, "core/shared/utils/uncommon");

    println!(
        "cargo:rustc-link-search=native={}",
        repo_root().join("build").to_str().unwrap()
    );
    println!("cargo:rustc-link-lib=static=vmlib");

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
    }

    println!("cargo:rustc-link-lib=dylib=c++");
    println!("cargo:rustc-link-lib=dylib=z");
    println!("cargo:rustc-link-lib=dylib=zstd");
    println!("cargo:rustc-link-lib=dylib=c");
    println!("cargo:rustc-link-lib=dylib=m");
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rustc-link-lib=dylib=pthread");

    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-lib=dylib=rt");

    let bindings = builder
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .derive_default(true)
        .trust_clang_mangling(false)
        .layout_tests(false)
        .generate_comments(true)
        .generate()
        .expect("Unable to generate bindings");

    let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
