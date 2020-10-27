FROM rust:1.46-alpine as builder

RUN apk add libc-dev && apk update

WORKDIR /usr/src/prio-server

# First, copy just the Cargo.toml and a dummy main, then build them.
# This primes a layer that contains the built dependencies.
COPY facilitator/Cargo.lock facilitator/Cargo.lock
COPY facilitator/Cargo.toml facilitator/Cargo.toml
RUN sed -i /build.rs/d facilitator/Cargo.toml
RUN mkdir -p facilitator/src
RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > facilitator/src/main.rs
WORKDIR facilitator
RUN cargo build --release --manifest-path ./facilitator/Cargo.toml

# Clean up and copy the real source.
#RUN rm -f facilitator/target/*/release/deps/facilitator* facilitator/src/main.rs
RUN rm -f facilitator/src/main.rs

# We enumerate these paths so that `docker build` fails in an obvious way if run
# from the wrong place.
COPY ./avro-schema ./avro-schema
COPY ./facilitator ./facilitator
RUN cargo install --path ./facilitator

FROM rust:1.46-alpine
RUN apk update
COPY --from=builder /usr/local/cargo/bin/facilitator /usr/local/bin/facilitator
ENTRYPOINT ["/usr/local/bin/facilitator"]
