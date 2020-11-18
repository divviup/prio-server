FROM rust:1.46-alpine as builder

RUN apk add libc-dev && apk update
# Attempt to install a nonexistent package. This triggers
# updating the crates.io index separately from building the
# dependencies, so if dependencies change we don't have to
# re-download the whole index.
RUN cargo install _update_crates_io_failure_is_expected_ ; true

WORKDIR /usr/src/prio-server

# First, copy just the Cargo.toml and a dummy main, then build them.
# This primes a layer that contains the built dependencies.
COPY facilitator/Cargo.lock facilitator/Cargo.lock
COPY facilitator/Cargo.toml facilitator/Cargo.toml
RUN sed -i /build.rs/d facilitator/Cargo.toml
RUN mkdir -p facilitator/src
RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > facilitator/src/main.rs
# This cargo build command must match the one below, or the build cache will not be reused.
RUN cargo build --manifest-path ./facilitator/Cargo.toml

# Clean up and copy the real source.
# After this we have a layer that should be cacheable so long as the dependencies don't change.
RUN rm -f facilitator/target/*/release/deps/facilitator* facilitator/src/main.rs

# We enumerate these paths so that `docker build` fails in an obvious way if run
# from the wrong place.
COPY ./avro-schema ./avro-schema
COPY ./facilitator ./facilitator
# This cargo build command must match the one above, or the build cache will not be reused.
RUN cargo build --manifest-path ./facilitator/Cargo.toml
# We build in debug mode so the build runs quickly, then strip the binary for size.
RUN strip facilitator/target/debug/facilitator

# Build a minimal container containing only the binary, the one .so it needs, and root certs.
RUN cp /usr/src/prio-server/facilitator/target/debug/facilitator /facilitator
ENTRYPOINT ["/facilitator"]
