FROM golang:1.20.4 as builder
# set GOPATH to empty since we're building with modules.
ENV GOPATH=
WORKDIR /workspace/
COPY go.mod go.mod
COPY go.sum go.sum
RUN go mod download
COPY . .
ARG BUILD_INFO=unspecified
RUN \
  CGO_ENABLED=0 \
  GOOS=linux \
  go build -ldflags="-w -X 'main.BuildInfo=${BUILD_INFO}'" -o workflow-manager ./

FROM scratch
ARG BUILD_INFO=unspecified
LABEL build_info="${BUILD_INFO}"
# The container needs a copy of trusted roots. Copy them from the builder image.
# See
# https://medium.com/@kelseyhightower/optimizing-docker-images-for-static-binaries-b5696e26eb07
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /workspace/workflow-manager workflow-manager
ENTRYPOINT ["/workflow-manager"]
