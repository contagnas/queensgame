load(
    "//bazel/rules:rust.bzl",
    _optimized_wasm_bindgen = "optimized_wasm_bindgen",
    _release_binary = "release_binary",
    _rust_binary = "binary",
    _rust_image = "image",
    _rust_library = "library",
    _rust_shared_library = "shared_library",
    _rust_test = "test",
    _rust_wasm_bindgen = "wasm_bindgen",
    _rust_wasm_bindgen_toolchain = "wasm_bindgen_toolchain",
)

rust = struct(
    binary = _rust_binary,
    image = _rust_image,
    library = _rust_library,
    optimized_wasm_bindgen = _optimized_wasm_bindgen,
    release_binary = _release_binary,
    shared_library = _rust_shared_library,
    test = _rust_test,
    wasm_bindgen = _rust_wasm_bindgen,
    wasm_bindgen_toolchain = _rust_wasm_bindgen_toolchain,
)
