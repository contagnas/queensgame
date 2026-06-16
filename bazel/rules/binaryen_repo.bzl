_BINARYEN_VERSION = "130"

_BINARYEN_RELEASES = {
    "aarch64": struct(
        platform = "aarch64-linux",
        sha256 = "e6ae6e09ac40f4e14bc5be6f687c58e2995c84170013975fa641809dd3b480a0",
    ),
    "amd64": struct(
        platform = "x86_64-linux",
        sha256 = "0a18362361ad05465118cd8eeb72edaeec89de6894bc283576ef4e07aa3babcc",
    ),
    "x86_64": struct(
        platform = "x86_64-linux",
        sha256 = "0a18362361ad05465118cd8eeb72edaeec89de6894bc283576ef4e07aa3babcc",
    ),
}

def _binaryen_repository_impl(repository_ctx):
    release = _BINARYEN_RELEASES.get(repository_ctx.os.arch)
    if not release:
        fail("Unsupported Binaryen host architecture: %s" % repository_ctx.os.arch)

    archive = "binaryen-version_%s-%s.tar.gz" % (_BINARYEN_VERSION, release.platform)
    repository_ctx.download_and_extract(
        sha256 = release.sha256,
        stripPrefix = "binaryen-version_%s" % _BINARYEN_VERSION,
        url = "https://github.com/WebAssembly/binaryen/releases/download/version_%s/%s" % (
            _BINARYEN_VERSION,
            archive,
        ),
    )
    repository_ctx.file(
        "BUILD.bazel",
        """package(default_visibility = ["//visibility:public"])

exports_files(["bin/wasm-opt"])
""",
    )

binaryen_repository = repository_rule(
    implementation = _binaryen_repository_impl,
)
