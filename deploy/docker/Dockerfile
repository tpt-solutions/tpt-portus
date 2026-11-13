# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Skeleton image for tpt-portus (DESIGN.md §12). Not yet production-ready:
# the crate currently ships a library only, so there is no binary to copy into
# the runtime stage. Add a `[[bin]]` target before using this image.

FROM rust:1-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime
RUN useradd --system --uid 10001 --no-create-home portus
COPY portus.toml /etc/tpt/portus.toml
# COPY --from=builder /build/target/release/tpt-portus /usr/local/bin/tpt-portus
USER 10001
EXPOSE 9400
ENV PORTUS_CONFIG=/etc/tpt/portus.toml
ENTRYPOINT ["/usr/local/bin/tpt-portus"]
