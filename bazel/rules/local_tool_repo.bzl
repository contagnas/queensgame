def _local_tool_repository_impl(repository_ctx):
    tool_path = repository_ctx.which(repository_ctx.attr.tool)
    if tool_path == None:
        fail("could not find %r on PATH" % repository_ctx.attr.tool)

    repository_ctx.symlink(tool_path, repository_ctx.attr.tool)
    repository_ctx.file(
        "BUILD.bazel",
        "exports_files([\"%s\"])\n" % repository_ctx.attr.tool,
    )

local_tool_repository = repository_rule(
    implementation = _local_tool_repository_impl,
    attrs = {
        "tool": attr.string(mandatory = True),
    },
)
