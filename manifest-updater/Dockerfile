FROM golang:1.17-alpine AS builder
# Get latest ca certs
RUN apk --update add ca-certificates

# Copy go modules & in-repo dependencies first
WORKDIR /workspace
RUN mkdir manifest-updater key-rotator
COPY key-rotator/ key-rotator/
COPY manifest-updater/go.mod manifest-updater/
COPY manifest-updater/go.sum manifest-updater/

WORKDIR /workspace/manifest-updater
# cache deps before building and copying source so that we don't need to re-download as much
# and so that source changes don't invalidate our downloaded layer
RUN go mod download
# Copy entire directory over
COPY manifest-updater/ .

# Build
RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o updater main.go

FROM scratch
WORKDIR /

# Bring ca certs
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /workspace/manifest-updater/updater .

ENTRYPOINT ["/updater"]

