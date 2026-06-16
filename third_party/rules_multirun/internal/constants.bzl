"""Internal constants shared between multirun rules."""

CommandInfo = provider(
    fields = ["description"],
    doc = "Information about commands used by their multirun.",
)

RUNFILES_PREFIX = """#!/usr/bin/env bash

# --- begin runfiles.bash initialization v2 ---
# Copy-pasted from the Bazel Bash runfiles library v2.
set -uo pipefail; f=bazel_tools/tools/bash/runfiles/runfiles.bash
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \\
 source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \\
 source "$0.runfiles/$f" 2>/dev/null || \\
 source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \\
 source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \\
 { echo>&2 "ERROR: cannot find $f"; exit 1; }; f=; set -e
# --- end runfiles.bash initialization v2 ---

# Export RUNFILES_* envvars (and a couple more) for subprocesses.
runfiles_export_envvars

"""

def update_attrs(attrs, cfg, allowlist):
    """Add transition allowlist attributes when needed."""
    if type(cfg) == "transition":
        attrs["_allowlist_function_transition"] = attr.label(default = allowlist or "@bazel_tools//tools/allowlists/function_transition_allowlist")

    return attrs

def rlocation_path(ctx, file):
    """Produce the runfiles lookup path for the given file."""
    if file.short_path.startswith("../"):
        return file.short_path[3:]
    return ctx.workspace_name + "/" + file.short_path
