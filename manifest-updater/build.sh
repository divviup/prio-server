#!/bin/bash -eux
cd $(dirname $0)

# Embed info about the build.
COMMIT_ID="$(git rev-parse --short=8 HEAD)"
BUILD_ID="$(git symbolic-ref --short HEAD 2>/dev/null || true)+${COMMIT_ID}"
BUILD_TIME="$(date)"
BUILD_INFO="${BUILD_ID} - ${BUILD_TIME}"

cd ..

docker build -f manifest-updater/Dockerfile --tag letsencrypt/prio-manifest-updater --build-arg BUILD_INFO="${BUILD_INFO}" .
