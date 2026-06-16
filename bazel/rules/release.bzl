def _release_transition_impl(_settings, _attr):
    return {
        "//command_line_option:compilation_mode": "opt",
    }

_release_transition = transition(
    implementation = _release_transition_impl,
    inputs = [],
    outputs = [
        "//command_line_option:compilation_mode",
    ],
)

def _release_binary_impl(ctx):
    binary = ctx.executable.binary
    out = ctx.actions.declare_file(ctx.label.name)
    patchelf = ctx.executable._patchelf if ctx.attr.interpreter else None

    ctx.actions.run_shell(
        inputs = [binary],
        outputs = [out],
        tools = [patchelf] if patchelf else [],
        use_default_shell_env = True,
        command = """
set -euo pipefail

cp "$1" "$2"
chmod 0755 "$2"

if [ -n "$3" ]; then
  "$4" --set-interpreter "$3" "$2"
fi
""",
        arguments = [
            binary.path,
            out.path,
            ctx.attr.interpreter,
            patchelf.path if patchelf else "",
        ],
    )

    return [DefaultInfo(
        executable = out,
        files = depset([out]),
    )]

release_binary = rule(
    implementation = _release_binary_impl,
    attrs = {
        "binary": attr.label(
            cfg = _release_transition,
            executable = True,
            mandatory = True,
        ),
        "interpreter": attr.string(),
        "_patchelf": attr.label(
            allow_single_file = True,
            cfg = "exec",
            default = "@patchelf//:patchelf",
            executable = True,
        ),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    },
    executable = True,
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
            cfg = _release_transition,
            mandatory = True,
        ),
        "wasm_opt": attr.label(
            allow_single_file = True,
            mandatory = True,
        ),
        "wasm_opt_flags": attr.string_list(default = ["-Oz"]),
        "out_dir": attr.string(),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    },
)
