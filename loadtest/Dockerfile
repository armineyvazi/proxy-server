FROM golang:1.26-alpine AS builder
WORKDIR /build
COPY go.mod ./
COPY *.go ./
RUN go build -o /loadtest .

FROM alpine:3.20
COPY --from=builder /loadtest /usr/local/bin/loadtest
EXPOSE 8090
ENTRYPOINT ["/usr/local/bin/loadtest"]
