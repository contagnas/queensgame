"""Run multiple command targets through a small Rust runner."""

load("@bazel_skylib//lib:shell.bzl", "shell")
load(
    "//internal:constants.bzl",
    "CommandInfo",
    "RUNFILES_PREFIX",
    "rlocation_path",
    "update_attrs",
)

_BinaryArgsEnvInfo = provider(
    fields = ["args", "env"],
    doc = "The arguments and environment to use when running the binary.",
)

def _bool(value):
    return "1" if value else "0"

def _escape(value):
    return value.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")

def _line(key, value):
    return "{}\t{}".format(key, _escape(value))

def _binary_args_env_aspect_impl(target, ctx):
    if _BinaryArgsEnvInfo in target:
        return []

    is_executable = target.files_to_run != None and target.files_to_run.executable != None
    args = getattr(ctx.rule.attr, "args", [])
    env = dict(getattr(ctx.rule.attr, "env", {}))

    if RunEnvironmentInfo in target:
        env.update(target[RunEnvironmentInfo].environment)

    if is_executable and (args or env):
        expansion_targets = getattr(ctx.rule.attr, "data", [])
        if expansion_targets:
            args = [
                ctx.expand_location(arg, expansion_targets)
                for arg in args
            ]
            env = {
                name: ctx.expand_location(val, expansion_targets)
                for name, val in env.items()
            }
        return [_BinaryArgsEnvInfo(args = args, env = env)]

    return []

_binary_args_env_aspect = aspect(
    implementation = _binary_args_env_aspect_impl,
)

def _multirun_impl(ctx):
    instructions_file = ctx.actions.declare_file(ctx.label.name + ".instructions")
    runner_info = ctx.attr._runner[DefaultInfo]
    runner_exe = runner_info.files_to_run.executable

    runfiles = ctx.runfiles(files = [instructions_file, runner_exe])
    runfiles = runfiles.merge(ctx.attr._bash_runfiles[DefaultInfo].default_runfiles)
    runfiles = runfiles.merge(runner_info.default_runfiles)

    for data_dep in ctx.attr.data:
        default_runfiles = data_dep[DefaultInfo].default_runfiles
        if default_runfiles != None:
            runfiles = runfiles.merge(default_runfiles)

    commands = []
    runfiles_files = []
    for command in ctx.attr.commands:
        default_info = command[DefaultInfo]
        if default_info.files_to_run == None:
            fail("{} is not executable".format(command.label), attr = "commands")
        exe = default_info.files_to_run.executable
        if exe == None:
            fail("{} does not have an executable file".format(command.label), attr = "commands")
        runfiles_files.append(exe)

        args = []
        env = {}
        if _BinaryArgsEnvInfo in command:
            args = command[_BinaryArgsEnvInfo].args
            env = command[_BinaryArgsEnvInfo].env

        default_runfiles = default_info.default_runfiles
        if default_runfiles != None:
            runfiles = runfiles.merge(default_runfiles)

        tag = command[CommandInfo].description if CommandInfo in command else "Running {}".format(command.label)
        commands.append(struct(
            args = args,
            env = env,
            path = exe.short_path,
            tag = tag,
        ))

    if ctx.attr.jobs < 0:
        fail("'jobs' attribute should be at least 0")
    if ctx.attr.jobs > 0 and ctx.attr.forward_stdin:
        fail("'forward_stdin' can only apply to parallel jobs ('jobs' === 0)")

    lines = [
        "rules_multirun\t1",
        _line("workspace", ctx.workspace_name),
        _line("jobs", str(ctx.attr.jobs)),
        _line("print_command", _bool(ctx.attr.print_command)),
        _line("keep_going", _bool(ctx.attr.keep_going)),
        _line("buffer_output", _bool(ctx.attr.buffer_output)),
        _line("forward_stdin", _bool(ctx.attr.forward_stdin)),
    ]
    for command in commands:
        lines.append("command")
        lines.append(_line("tag", command.tag))
        lines.append(_line("path", command.path))
        for arg in command.args:
            lines.append(_line("arg", arg))
        for key, value in command.env.items():
            lines.append("env\t{}\t{}".format(_escape(key), _escape(value)))
        lines.append("end")

    ctx.actions.write(
        output = instructions_file,
        content = "\n".join(lines) + "\n",
    )

    script = """\
multirun_script="$(rlocation {})"
instructions="$(rlocation {})"
exec "$multirun_script" "$instructions" "$@"
""".format(shell.quote(rlocation_path(ctx, runner_exe)), shell.quote(rlocation_path(ctx, instructions_file)))
    out_file = ctx.actions.declare_file(ctx.label.name + ".bash")
    ctx.actions.write(
        output = out_file,
        content = RUNFILES_PREFIX + script,
        is_executable = True,
    )
    return [
        DefaultInfo(
            files = depset([out_file]),
            runfiles = runfiles.merge(ctx.runfiles(files = runfiles_files + ctx.files.data)),
            executable = out_file,
        ),
    ]

def multirun_with_transition(cfg, allowlist = None):
    """Creates a multirun rule which transitions all commands to the given configuration."""
    attrs = {
        "commands": attr.label_list(
            mandatory = False,
            allow_files = True,
            aspects = [_binary_args_env_aspect],
            doc = "Targets to run.",
            cfg = cfg,
        ),
        "data": attr.label_list(
            doc = "Runtime data needed by commands.",
            allow_files = True,
        ),
        "jobs": attr.int(
            default = 1,
            doc = "Maximum parallel commands. Set to 0 for unlimited parallelism.",
        ),
        "print_command": attr.bool(
            default = True,
            doc = "Print what command is being run.",
        ),
        "keep_going": attr.bool(
            default = False,
            doc = "Keep going after a command fails.",
        ),
        "buffer_output": attr.bool(
            default = False,
            doc = "Buffer command output and print it after each command finishes.",
        ),
        "forward_stdin": attr.bool(
            default = False,
            doc = "Forward stdin to child processes.",
        ),
        "_bash_runfiles": attr.label(
            default = Label("@bazel_tools//tools/bash/runfiles"),
        ),
        "_runner": attr.label(
            default = Label("//internal:multirun"),
            cfg = "target",
            executable = True,
        ),
    }

    return rule(
        implementation = _multirun_impl,
        attrs = update_attrs(attrs, cfg, allowlist),
        executable = True,
    )

multirun = multirun_with_transition("target")
