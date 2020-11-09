FROM golang:1.15-alpine AS builder
# Get latest ca certs
RUN apk --update add ca-certificates

WORKDIR /workspace
# Copy entire directory over
COPY . .
# Cache the go modules as a build layer
RUN go mod download

# Build
RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o updater main.go

FROM scratch
WORKDIR /

# Bring ca certs
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /workspace/manager .

USER nonroot:nonroot

ENTRYPOINT ["/updater"]

