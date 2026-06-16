# Agent Notes

- Run Bazel with the same output base a developer will use. Do not add ad hoc `--output_base=...` flags for routine build, test, or run verification unless the user explicitly asks for that cache location. A custom output base creates a separate action cache, so successful verification there does not imply `bazelisk build/test/run ...` will be warm. Using a separate `--output_base=...` is OK when intentionally running multiple Bazel commands in parallel to avoid the Bazel server lock; call that out when reporting results.
