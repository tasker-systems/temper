# Throttled cold-load bench bed for issue #451.
#
# Reproduces the serverless embed under a Vercel-like constrained CPU so the cold model load can be
# characterized locally (see examples/coldload_probe.rs). This image carries ONLY the toolchain +
# ONNX Runtime; the repo source is bind-mounted at run time and the cargo target dir is a named
# volume, so an edit on the host is picked up by the next `run.sh` with no image rebuild.
#
# ONNX Runtime is pinned to the version the SERVER bundles (VERS_1.24.2 in
# lib/x86_64-unknown-linux-gnu/libonnxruntime.so), so the graph-optimization implementation under
# test matches the deploy. This bed is arm64-native (fast, clean numbers); the deploy is x86_64, so
# it tests the compute MECHANISM, not the exact silicon.
FROM rust:1-bookworm

ARG ORT_VERSION=1.24.2
ARG ORT_ARCH=aarch64

# ONNX Runtime shared library for ort's `load-dynamic`. Pulled from the official release so it is
# the same build the deploy uses, just the aarch64 flavor.
RUN set -eux; \
    curl -fsSL -o /tmp/ort.tgz \
      "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}.tgz"; \
    mkdir -p /opt/onnxruntime; \
    tar -xzf /tmp/ort.tgz -C /opt/onnxruntime --strip-components=1; \
    ln -s "$(ls /opt/onnxruntime/lib/libonnxruntime.so* | head -n1)" /usr/local/lib/libonnxruntime.so; \
    rm /tmp/ort.tgz

# `load-dynamic` reads this to find the runtime (see embed.rs init_ort_runtime non-Linux-bundled arm).
ENV ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
# Keep cargo's output off the read-only source mount and in the cache volume.
ENV CARGO_TARGET_DIR=/target
WORKDIR /repo

CMD ["bash"]
