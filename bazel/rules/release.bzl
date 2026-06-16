_EXTRA_RUSTC_FLAG = "@rules_rust//rust/settings:extra_rustc_flag"
_LTO = "@rules_rust//rust/settings:lto"
_WASM_SIZE_RUSTC_FLAGS = [
    "-Ccodegen-units=1",
    "-Copt-level=z",
]

def _wasm_release_transition_impl(settings, _attr):
    return {
        "//command_line_option:compilation_mode": "opt",
        _EXTRA_RUSTC_FLAG: settings[_EXTRA_RUSTC_FLAG] + _WASM_SIZE_RUSTC_FLAGS,
        _LTO: "thin",
    }

_wasm_release_transition = transition(
    implementation = _wasm_release_transition_impl,
    inputs = [
        _EXTRA_RUSTC_FLAG,
    ],
    outputs = [
        "//command_line_option:compilation_mode",
        _EXTRA_RUSTC_FLAG,
        _LTO,
    ],
)

def _optimized_wasm_bindgen_impl(ctx):
    out = ctx.actions.declare_directory(ctx.attr.out_dir or ctx.label.name)
    srcs = ctx.files.src
    flags = " ".join(["'%s'" % flag for flag in ctx.attr.wasm_opt_flags])

    ctx.actions.run_shell(
        inputs = srcs,
        outputs = [out],
        tools = [ctx.file.wasm_opt],
        use_default_shell_env = True,
        command = """
set -euo pipefail

out="$1"
wasm_opt="$2"
shift 2

rm -rf "$out"
mkdir -p "$out"

for src in "$@"; do
  cp -R "$src" "$out/$(basename "$src")"
done

find "$out" -type f -name '*.wasm' -print0 | while IFS= read -r -d '' wasm; do
  tmp="${wasm}.wasmopt"
  "$wasm_opt" %s -o "$tmp" "$wasm"
  mv "$tmp" "$wasm"
done
""" % flags,
        arguments = [out.path, ctx.file.wasm_opt.path] + [src.path for src in srcs],
    )

    return [DefaultInfo(files = depset([out]))]

optimized_wasm_bindgen = rule(
    implementation = _optimized_wasm_bindgen_impl,
    attrs = {
        "src": attr.label(
            cfg = _wasm_release_transition,
            mandatory = True,
        ),
        "wasm_opt": attr.label(
            allow_single_file = True,
            mandatory = True,
        ),
        "wasm_opt_flags": attr.string_list(default = ["-Oz", "--strip-debug", "--strip-producers"]),
        "out_dir": attr.string(),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    },
)
