FROM cgr.dev/chainguard/static

ARG GOARCH=amd64

COPY testbox-linux-$GOARCH /usr/local/bin/testbox

USER nonroot:nonroot

ENTRYPOINT ["/usr/local/bin/testbox"]
