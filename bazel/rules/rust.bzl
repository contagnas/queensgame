load(
    "@rules_rust//rust:defs.bzl",
    _rust_binary = "rust_binary",
    _rust_library = "rust_library",
    _rust_shared_library = "rust_shared_library",
    _rust_test = "rust_test",
)
load(
    "@rules_rust_wasm_bindgen//:defs.bzl",
    _wasm_bindgen = "rust_wasm_bindgen",
    _wasm_bindgen_toolchain = "rust_wasm_bindgen_toolchain",
)
load(
    "//bazel/rules:release.bzl",
    _optimized_wasm_bindgen = "optimized_wasm_bindgen",
    _release_binary = "release_binary",
)
load("//bazel/rules:rust_image.bzl", _image = "rust_binary_image")

DEFAULT_EDITION = "2024"

def _default_crate_name(name):
    return name.replace("-", "_")

def binary(name, srcs = ["main.rs"], crate_root = "main.rs", edition = DEFAULT_EDITION, **kwargs):
    _rust_binary(
        name = name,
        srcs = srcs,
        crate_root = crate_root,
        edition = edition,
        **kwargs
    )

def library(
        name,
        srcs = ["lib.rs"],
        crate_name = None,
        crate_root = "lib.rs",
        edition = DEFAULT_EDITION,
        **kwargs):
    _rust_library(
        name = name,
        srcs = srcs,
        crate_name = crate_name or _default_crate_name(name),
        crate_root = crate_root,
        edition = edition,
        **kwargs
    )

def shared_library(
        name,
        srcs = ["lib.rs"],
        crate_name = None,
        crate_root = "lib.rs",
        edition = DEFAULT_EDITION,
        **kwargs):
    _rust_shared_library(
        name = name,
        srcs = srcs,
        crate_name = crate_name or _default_crate_name(name),
        crate_root = crate_root,
        edition = edition,
        **kwargs
    )

def test(
        name,
        crate = None,
        srcs = None,
        crate_root = None,
        edition = DEFAULT_EDITION,
        **kwargs):
    if crate != None:
        if srcs != None:
            fail("rust.test cannot set both crate and srcs")
        if crate_root != None:
            fail("rust.test cannot set both crate and crate_root")
        _rust_test(
            name = name,
            crate = crate,
            edition = edition,
            **kwargs
        )
        return

    _rust_test(
        name = name,
        srcs = srcs if srcs != None else ["main.rs"],
        crate_root = crate_root if crate_root != None else "main.rs",
        edition = edition,
        **kwargs
    )

image = _image
optimized_wasm_bindgen = _optimized_wasm_bindgen
release_binary = _release_binary
wasm_bindgen = _wasm_bindgen
wasm_bindgen_toolchain = _wasm_bindgen_toolchain
