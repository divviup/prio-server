FROM scratch
COPY ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY workflow-manager workflow-manager
ENTRYPOINT ["/workflow-manager"]
