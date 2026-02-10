FROM --platform=$BUILDPLATFORM cgr.dev/chainguard/go:latest AS builder

WORKDIR /app

COPY go.mod go.sum ./
RUN go mod download
COPY . .

ARG TARGETOS TARGETARCH
RUN GOOS=$TARGETOS GOARCH=$TARGETARCH CGO_ENABLED=0 go build -a -o _output/server

FROM cgr.dev/chainguard/static

COPY --from=builder ./app/_output/server /server

USER nonroot:nonroot

EXPOSE 18080
CMD ["./server"]
