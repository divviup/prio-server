#!/bin/bash -eux
# Embed info about the build.
COMMIT_ID="$(git rev-parse --short=8 HEAD)"
BUILD_ID="$(git symbolic-ref --short HEAD 2>/dev/null || true)+${COMMIT_ID}"
BUILD_TIME="$(date)"
export BUILD_INFO="${BUILD_ID} - ${BUILD_TIME}"

cd -- "$(dirname -- "$0")"/..

docker build --tag letsencrypt/prio-facilitator -f facilitator/Dockerfile --build-arg BUILD_INFO="${BUILD_INFO}" .
