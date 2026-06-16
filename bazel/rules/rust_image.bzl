load("@rules_img//img:image.bzl", "image_manifest")
load("@rules_img//img:layer.bzl", "file_metadata", "image_layer")
load("@rules_img//img:load.bzl", "image_load")
load("@rules_img_images.bzl", "image")
load("//bazel/rules:release.bzl", "release_binary")

def _check_container_path(path, attr):
    if not path.startswith("/"):
        fail("%s must use absolute container paths, got %r" % (attr, path))

def rust_binary_image(
        name,
        binary,
        binary_path,
        files = None,
        env = None,
        tag = None,
        base = None,
        platform = "//platforms:linux_amd64",
        user = None,
        uid = 65532,
        gid = 65532,
        interpreter = "/lib64/ld-linux-x86-64.so.2",
        visibility = None):
    """Builds a production OCI image around an optimized Rust binary."""
    _check_container_path(binary_path, "binary_path")

    release_name = name + ".binary"
    layer_name = name + ".layer"
    load_name = name + ".load"

    release_binary(
        name = release_name,
        binary = binary,
        interpreter = interpreter,
        visibility = ["//visibility:private"],
    )

    layer_srcs = {
        binary_path: ":" + release_name,
    }
    if files:
        for path, src in files.items():
            _check_container_path(path, "files")
            if path == binary_path:
                fail("files must not overwrite binary_path %r" % binary_path)
            layer_srcs[path] = src

    image_layer(
        name = layer_name,
        srcs = layer_srcs,
        compress = "gzip",
        default_metadata = file_metadata(
            gid = gid,
            uid = uid,
        ),
        file_metadata = {
            binary_path: file_metadata(
                gid = gid,
                mode = "0755",
                uid = uid,
            ),
        },
        include_runfiles = False,
        visibility = ["//visibility:private"],
    )

    image_manifest(
        name = name,
        base = base or image("distroless_cc_nonroot"),
        entrypoint = [binary_path],
        env = env or {},
        layers = [":" + layer_name],
        platform = platform,
        user = user or ("%s:%s" % (uid, gid)),
        visibility = visibility,
    )

    image_load(
        name = load_name,
        image = ":" + name,
        tag = tag or (name + ":latest"),
        visibility = visibility,
    )
