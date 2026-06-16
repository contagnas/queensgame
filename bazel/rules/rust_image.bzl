load("@rules_img//img:image.bzl", "image_manifest")
load("@rules_img//img:layer.bzl", "file_metadata", "image_layer")
load("@rules_img//img:load.bzl", "image_load")
load("@rules_img_images.bzl", "image")

def _check_container_path(path, attr):
    if not path.startswith("/"):
        fail("%s must use absolute container paths, got %r" % (attr, path))

def _label_name(label):
    label = str(label)
    if label.startswith("select("):
        fail("binary_path is required when binary uses select()")

    if ":" in label:
        name = label.rsplit(":", 1)[1]
    else:
        name = label.rsplit("/", 1)[-1]

    if not name:
        fail("could not infer binary_path from binary label %r" % label)

    return name

def _default_binary_path(binary):
    return "/app/bin/" + _label_name(binary)

def rust_binary_image(
        name,
        binary,
        binary_path = None,
        files = None,
        env = None,
        tag = None,
        base = None,
        platform = "//platforms:linux_amd64",
        user = None,
        uid = 65532,
        gid = 65532,
        visibility = None):
    """Builds an OCI image around a Rust binary."""
    binary_path = binary_path or _default_binary_path(binary)
    _check_container_path(binary_path, "binary_path")

    layer_name = name + ".layer"
    files_layer_name = name + ".files_layer"
    load_name = name + ".load"

    if type(files) == "dict":
        for path, src in files.items():
            _check_container_path(path, "files")
            if path == binary_path:
                fail("files must not overwrite binary_path %r" % binary_path)

    image_layer(
        name = layer_name,
        srcs = {
            binary_path: binary,
        },
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

    layers = [":" + layer_name]
    if files != None:
        image_layer(
            name = files_layer_name,
            srcs = files,
            compress = "gzip",
            default_metadata = file_metadata(
                gid = gid,
                uid = uid,
            ),
            include_runfiles = False,
            visibility = ["//visibility:private"],
        )
        layers.append(":" + files_layer_name)

    image_manifest(
        name = name,
        base = base or image("distroless_cc_nonroot"),
        entrypoint = [binary_path],
        env = env or {},
        layers = layers,
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
