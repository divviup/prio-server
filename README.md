# prio-server
[![Build Status]][actions]

[Build Status]: https://github.com/abetterinternet/prio-server/workflows/ci-build/badge.svg
[actions]: https://github.com/abetterinternet/prio-server/actions?query=branch%3Amain

This is ISRG's implementation server components for [Prio](https://crypto.stanford.edu/prio/), the privacy preserving statistics aggregation system.

# `prio-server` is no longer supported

On May 11, 2023, the United States federal government's Public Health Emergency declaration for COVID-19 ended ([CDC](https://www.cdc.gov/coronavirus/2019-ncov/your-health/end-of-phe.html)), which means that the [Exposure Notifications Private Analytics](https://divviup.org/blog/prio-services-for-covid-en/) system that `prio-server` powers is being turned off.

This project is no longer maintained by [ISRG](https://abetterinternet.org). No new features will be added and no bug or security fixes will be made. This repository will remain on GitHub in the spirit of transparency and in case it is of interest to researchers.

If you are interested in Prio or privacy-preserving aggregation of telemetry, check out [Divvi Up](https://divviup.org) or other implementations of the more secure and versatile [Distributed Aggregation Protocol](https://datatracker.ietf.org/doc/draft-ietf-ppm-dap/), like [Janus](https://github.com/divviup/janus).

---

`avro-schema` contains [Avro](https://avro.apache.org/docs/current/index.html) schema definitions for interoperation with other actors in the Prio system. `facilitator` contains the Rust implementation of ISRG's Prio facilitation server. `terraform` contains a Terraform module for deploying data share processor servers.

## Prio share processor workflow

![Prio workflow diagram](docs/prio-workflow.gv.svg)

This GitHub project implements the "facilitator" box in the diagram.

## Releases

We use a GitHub Action to build Docker images and push them to [DockerHub](https://hub.docker.com/repository/docker/letsencrypt/prio-facilitator). To cut a release and push, publish a release in [GitHub's releases UI](https://github.com/abetterinternet/prio-server/releases/new). Docker images will be automatically generated and pushed to DockerHub.

## Metrics and monitoring

`prio-server` uses Prometheus for metrics and alerting. See [documentation for more information](docs/monitoring.md).
