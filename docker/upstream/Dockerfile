FROM golang:1.26-alpine AS builder

WORKDIR /build
COPY go.mod ./
COPY upstream.go ./
RUN go build -o /upstream .

FROM alpine:3.20
COPY --from=builder /upstream /usr/local/bin/upstream
EXPOSE 9000
ENTRYPOINT ["/usr/local/bin/upstream"]
