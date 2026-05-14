FROM golang:1.25-alpine AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY main.go .
RUN CGO_ENABLED=0 go build -o ext-proc-bbr .

FROM alpine:3.21
COPY --from=builder /app/ext-proc-bbr /usr/local/bin/
ENTRYPOINT ["ext-proc-bbr"]
