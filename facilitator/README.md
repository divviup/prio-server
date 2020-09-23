# facilitator

This is ISRG's implementation of a Prio accumulation server. It ingests a share of data uploaded by a device, validates it against other facilitators, then emits the accumulated share to a Prio aggregator.

## Getting started

[Install a Rust toolchain](https://www.rust-lang.org/tools/install), then just `cargo build|run|test`. See `cargo run -- --help` for information on the various options and subcommands.

## Generating ingestion data

To generate sample ingestion data, see the `generate-ingestion-sample` command and its usage (`cargo run -- generate-ingestion-sample --help`).

## Docker

To build a Docker image, try `docker build -t my-image-repository/facilitator:x.y.z -f facilitator/Dockerfile .` *from the root directory of `prio-server`*. This is important because building `facilitator` depends on the schema files in `avro-schema`.

## References

[Prio Data Share Batch IDL](https://docs.google.com/document/d/1L06dpE7OcC4CXho2UswrfHrnWKtbA9aSSmO_5o7Ku6I/edit#heading=h.3kq1yexquq2g)
[ISRG Prio server-side components design doc](https://docs.google.com/document/d/1MdfM3QT63ISU70l63bwzTrxr93Z7Tv7EDjLfammzo6Q/edit#)
