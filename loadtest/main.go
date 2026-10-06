// Command loadtest is an HTTP load generator with a web UI and Prometheus
// metrics, used to benchmark the proxy-server against nginx.
//
//	POST /api/run     start a test    (RunConfig JSON body)
//	POST /api/stop    abort the test
//	POST /api/reset   clear all stats
//	GET  /api/status  live stats (JSON)
//	GET  /api/history time-series points for the UI charts
//	GET  /api/targets configured targets
//	GET  /metrics     Prometheus text format
//	GET  /            web UI
package main

import (
	"log"
	"net/http"
)

const listenAddr = ":8090"

func main() {
	initTargets()

	http.HandleFunc("/", handleUI)
	http.HandleFunc("/api/targets", handleTargets)
	http.HandleFunc("/api/run", handleRun)
	http.HandleFunc("/api/stop", handleStop)
	http.HandleFunc("/api/reset", handleReset)
	http.HandleFunc("/api/status", handleStatus)
	http.HandleFunc("/api/history", handleHistory)
	http.HandleFunc("/metrics", handleMetrics)

	log.Printf("load tester listening on %s", listenAddr)
	log.Printf("  UI       http://localhost%s/", listenAddr)
	log.Printf("  metrics  http://localhost%s/metrics", listenAddr)
	log.Fatal(http.ListenAndServe(listenAddr, nil))
}
