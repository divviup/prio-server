FROM golang:1.20.3 as builder

# Copy go modules first, to allow module download to be cached.
WORKDIR /workspace
COPY key-rotator/go.mod go.mod
COPY key-rotator/go.sum go.sum
RUN go mod download

# Copy everything over and build.
COPY key-rotator/. .
RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o key-rotator

FROM scratch
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /workspace/key-rotator .
ENTRYPOINT ["/key-rotator"]
