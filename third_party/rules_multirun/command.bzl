"""Simple command wrapper rule used by multirun."""

load("@bazel_skylib//lib:shell.bzl", "shell")
load(
    "//internal:constants.bzl",
    "CommandInfo",
    "RUNFILES_PREFIX",
    "rlocation_path",
    "update_attrs",
)

def _force_opt_impl(_settings, _attr):
    return {"//command_line_option:compilation_mode": "opt"}

_force_opt = transition(
    implementation = _force_opt_impl,
    inputs = [],
    outputs = ["//command_line_option:compilation_mode"],
)

def _expand_and_quote(*, ctx, attr, string, targets):
    expanded = ctx.expand_make_variables(
        attr,
        ctx.expand_location(string, targets = targets),
        {},
    )

    if expanded.startswith("$(rlocation "):
        return "\"{}\"".format(expanded)
    return shell.quote(expanded)

def _command_impl(ctx):
    runfiles = ctx.runfiles().merge(ctx.attr._bash_runfiles[DefaultInfo].default_runfiles)

    for data_dep in ctx.attr.data:
        default_runfiles = data_dep[DefaultInfo].default_runfiles
        if default_runfiles != None:
            runfiles = runfiles.merge(default_runfiles)

    command = ctx.attr.command if type(ctx.attr.command) == "Target" else ctx.attr.command[0]
    default_info = command[DefaultInfo]
    executable = default_info.files_to_run.executable

    default_runfiles = default_info.default_runfiles
    if default_runfiles != None:
        runfiles = runfiles.merge(default_runfiles)

    expansion_targets = ctx.attr.data

    str_env = [
        "export %s=%s" % (
            k,
            _expand_and_quote(
                ctx = ctx,
                attr = "environment",
                string = v,
                targets = expansion_targets,
            ),
        )
        for k, v in ctx.attr.environment.items()
    ]
    str_args = [
        _expand_and_quote(ctx = ctx, attr = "arguments", string = v, targets = expansion_targets)
        for v in ctx.attr.arguments
    ]
    cd_command = ""
    if ctx.attr.run_from_workspace_root:
        cd_command = 'cd "$BUILD_WORKSPACE_DIRECTORY"'
    command_exec = " ".join(["exec $(rlocation %s)" % shell.quote(rlocation_path(ctx, executable))] + str_args + ['"$@"\n'])

    out_file = ctx.actions.declare_file(ctx.label.name + ".bash")
    ctx.actions.write(
        output = out_file,
        content = "\n".join([RUNFILES_PREFIX] + str_env + [cd_command, command_exec]),
        is_executable = True,
    )

    providers = [
        DefaultInfo(
            files = depset([out_file]),
            runfiles = runfiles.merge(ctx.runfiles(files = ctx.files.data + [executable])),
            executable = out_file,
        ),
    ]

    if ctx.attr.description:
        providers.append(CommandInfo(description = ctx.attr.description))

    return providers

def command_with_transition(cfg, allowlist = None, doc = None):
    """Create a command rule with a transition to the given configuration."""
    attrs = {
        "arguments": attr.string_list(
            doc = "List of command line arguments. Subject to $(location) expansion.",
        ),
        "data": attr.label_list(
            doc = "Runtime data needed by this command.",
            allow_files = True,
        ),
        "environment": attr.string_dict(
            doc = "Environment variables. Subject to $(location) expansion.",
        ),
        "command": attr.label(
            mandatory = True,
            allow_files = True,
            executable = True,
            doc = "Target to run.",
            cfg = cfg,
        ),
        "description": attr.string(
            doc = "Description printed during multiruns.",
        ),
        "run_from_workspace_root": attr.bool(
            default = False,
            doc = "Run the command from the workspace root.",
        ),
        "_bash_runfiles": attr.label(
            default = Label("@bazel_tools//tools/bash/runfiles"),
        ),
    }

    return rule(
        implementation = _command_impl,
        attrs = update_attrs(attrs, cfg, allowlist),
        executable = True,
        doc = doc,
    )

command = command_with_transition("target")
command_force_opt = command_with_transition(_force_opt)
