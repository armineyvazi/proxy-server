// Minimal HTTP/1.1 upstream server used for proxy benchmarking.
// Responds to every request with 200 OK and a tiny body.
// Uses Go's built-in HTTP server which handles concurrency internally.
package main

import (
	"flag"
	"fmt"
	"log"
	"net/http"
	"runtime"
	"time"
)

func main() {
	addr := flag.String("addr", "127.0.0.1:9000", "listen address")
	flag.Parse()

	mux := http.NewServeMux()
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		w.Header().Set("X-Served-At", time.Now().Format(time.RFC3339Nano))
		fmt.Fprintf(w, "upstream OK: %s %s\n", r.Method, r.URL.Path)
	})

	srv := &http.Server{
		Addr:         *addr,
		Handler:      mux,
		ReadTimeout:  10 * time.Second,
		WriteTimeout: 10 * time.Second,
		IdleTimeout:  120 * time.Second,
	}

	log.Printf("upstream server on %s  GOMAXPROCS=%d", *addr, runtime.GOMAXPROCS(0))
	log.Fatal(srv.ListenAndServe())
}
