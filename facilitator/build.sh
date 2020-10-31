#!/bin/bash -eux
# Change to the directory above this one so we can pull in avro-schema.
cd -- "$(dirname -- "$0")"/..

# The container needs a copy of trusted roots. Try to copy them from the OS.
# See
# https://medium.com/@kelseyhightower/optimizing-docker-images-for-static-binaries-b5696e26eb07
# and
# https://github.com/golang/go/blob/master/src/crypto/x509/root_linux.go#L7
cp /etc/ssl/certs/ca-certificates.crt . \
 || cp /etc/pki/tls/certs/ca-bundle.crt ca-certificates.crt \
 || cp /etc/pki/tls/certs/ca-bundle.crt ca-certificates.crt
docker build --tag letsencrypt/prio-facilitator:latest -f facilitator/Dockerfile .
rm ca-certificates.crt
