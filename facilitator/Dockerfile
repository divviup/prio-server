FROM rust:1.46-alpine as builder

RUN apk add libc-dev && apk update

WORKDIR /usr/src/prio-server
# We enumerate these paths so that `docker build` fails in an obvious way if run
# from the wrong place.
COPY ./avro-schema ./avro-schema
COPY ./facilitator ./facilitator

RUN cargo install --path ./facilitator

FROM rust:1.46-alpine
RUN apk update
COPY --from=builder /usr/local/cargo/bin/facilitator /usr/local/bin/facilitator
ENTRYPOINT ["/usr/local/bin/facilitator"]
