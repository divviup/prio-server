# prio-server

This is ISRG's implementation server components for [Prio](https://crypto.stanford.edu/prio/), the privacy preserving statistics aggregation system.

`avro-schema` contains [Avro](https://avro.apache.org/docs/current/index.html) schema definitions for interoperation with other actors in the Prio system. `facilitator` contains the Rust implementation of ISRG's Prio facilitation server. `terraform` contains a Terraform module for deploying data share processor servers.

## Releases

We use a GitHub Action to build Docker images and push them to [DockerHub](https://hub.docker.com/repository/docker/letsencrypt/prio-facilitator). To cut a release and push:

- Bump the version number in `facilitator/Cargo.toml` and merge that change to `main`.
- Tag that commit on main, either in `git` or in [GitHub's releases UI](https://github.com/abetterinternet/prio-server/releases/new).
- Publish a release in [GitHub's releases UI](https://github.com/abetterinternet/prio-server/releases/new).
