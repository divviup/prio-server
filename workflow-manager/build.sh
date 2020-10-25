#!/bin/bash -eux
# The container needs a copy of trusted roots. Try to copy them from the OS.
# See
# https://medium.com/@kelseyhightower/optimizing-docker-images-for-static-binaries-b5696e26eb07
# and
# https://github.com/golang/go/blob/master/src/crypto/x509/root_linux.go#L7
cd $(dirname $0)
cp /etc/ssl/certs/ca-certificates.crt . \
 || cp /etc/pki/tls/certs/ca-bundle.crt ca-certificates.crt \
 || cp /etc/pki/tls/certs/ca-bundle.crt ca-certificates.crt
CGO_ENABLED=0 GOOS=linux go build -ldflags '-w' -o workflow-manager main.go
docker build --tag letsencrypt/prio-workflow-manager .
rm ca-certificates.crt
