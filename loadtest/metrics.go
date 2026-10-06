package main

import (
	"fmt"
	"net/http"
	"strings"
	"sync/atomic"
	"time"
)

// histBounds are the Prometheus histogram bucket upper-bounds, in seconds.
// The tail extends past 1s so histogram_quantile doesn't clamp p99 to the top
// bucket once the proxy starts shedding load under stress.
var histBounds = []float64{
	0.0005, 0.001, 0.002, 0.005, 0.010,
	0.020, 0.050, 0.100, 0.200, 0.500,
	1.000, 2.500, 5.000, 10.000,
}

func handleMetrics(w http.ResponseWriter, r *http.Request) {
	g.mu.Lock()
	state := g.state
	st := g.st
	logsSt := g.logsSt
	start := g.start
	cfg := g.cfg
	g.mu.Unlock()

	var sb strings.Builder
	running := 0
	if state == "running" {
		running = 1
	}
	fmt.Fprintf(&sb, "# HELP loadtest_running 1 while a load test is active\n")
	fmt.Fprintf(&sb, "# TYPE loadtest_running gauge\nloadtest_running %d\n\n", running)

	if st == nil {
		w.Header().Set("Content-Type", "text/plain; version=0.0.4")
		_, _ = w.Write([]byte(sb.String()))
		return
	}

	codes, errors, total, sorted := st.snapshot()
	elapsed := time.Since(start).Seconds()
	method := cfg.Method
	if method == "" {
		method = "GET"
	}
	scenario := cfg.ScenarioLabel
	if scenario == "" {
		scenario = "default"
	}

	fmt.Fprintf(&sb, "# HELP loadtest_requests_total Cumulative requests by HTTP status code\n")
	fmt.Fprintf(&sb, "# TYPE loadtest_requests_total counter\n")
	for code, count := range codes {
		fmt.Fprintf(&sb, "loadtest_requests_total{code=\"%d\",method=\"%s\",scenario=\"%s\"} %d\n", code, method, scenario, count)
	}
	if errors > 0 {
		fmt.Fprintf(&sb, "loadtest_requests_total{code=\"error\",method=\"%s\",scenario=\"%s\"} %d\n", method, scenario, errors)
	}
	fmt.Fprintf(&sb, "\n")

	// Split errors by source so a dashboard can answer "proxy or upstream?":
	//   network  = transport failure (refused/timeout/EOF) — no HTTP response
	//   gateway  = 502/503/504 from the proxy
	//   upstream = other 5xx passed through
	//   client   = 4xx
	var gatewayErrs, upstreamErrs, clientErrs int64
	for code, count := range codes {
		switch {
		case code == 502 || code == 503 || code == 504:
			gatewayErrs += count
		case code >= 500:
			upstreamErrs += count
		case code >= 400:
			clientErrs += count
		}
	}
	fmt.Fprintf(&sb, "# HELP loadtest_errors_by_type Cumulative errors categorized by source\n")
	fmt.Fprintf(&sb, "# TYPE loadtest_errors_by_type counter\n")
	fmt.Fprintf(&sb, "loadtest_errors_by_type{type=\"network\",scenario=\"%s\"} %d\n", scenario, errors)
	fmt.Fprintf(&sb, "loadtest_errors_by_type{type=\"gateway\",scenario=\"%s\"} %d\n", scenario, gatewayErrs)
	fmt.Fprintf(&sb, "loadtest_errors_by_type{type=\"upstream\",scenario=\"%s\"} %d\n", scenario, upstreamErrs)
	fmt.Fprintf(&sb, "loadtest_errors_by_type{type=\"client\",scenario=\"%s\"} %d\n", scenario, clientErrs)
	fmt.Fprintf(&sb, "\n")

	if cfg.RampMode {
		conc := atomic.LoadInt64(&g.currentConc)
		fmt.Fprintf(&sb, "# HELP loadtest_current_concurrency Live worker count in ramp mode\n")
		fmt.Fprintf(&sb, "# TYPE loadtest_current_concurrency gauge\nloadtest_current_concurrency %d\n\n", conc)
	}

	rps := 0.0
	if elapsed > 0 {
		rps = float64(total) / elapsed
	}
	fmt.Fprintf(&sb, "# HELP loadtest_rps Current requests per second\n")
	fmt.Fprintf(&sb, "# TYPE loadtest_rps gauge\nloadtest_rps %.2f\n\n", rps)

	// percentile() returns ms; divide once more for seconds.
	p50 := percentile(sorted, 50) / 1000.0
	p95 := percentile(sorted, 95) / 1000.0
	p99 := percentile(sorted, 99) / 1000.0
	fmt.Fprintf(&sb, "# HELP loadtest_latency_p50_seconds p50 latency\n# TYPE loadtest_latency_p50_seconds gauge\nloadtest_latency_p50_seconds %.6f\n\n", p50)
	fmt.Fprintf(&sb, "# HELP loadtest_latency_p95_seconds p95 latency\n# TYPE loadtest_latency_p95_seconds gauge\nloadtest_latency_p95_seconds %.6f\n\n", p95)
	fmt.Fprintf(&sb, "# HELP loadtest_latency_p99_seconds p99 latency\n# TYPE loadtest_latency_p99_seconds gauge\nloadtest_latency_p99_seconds %.6f\n\n", p99)

	fmt.Fprintf(&sb, "# HELP loadtest_latency_seconds HTTP request latency\n# TYPE loadtest_latency_seconds histogram\n")
	for _, bound := range histBounds {
		fmt.Fprintf(&sb, "loadtest_latency_seconds_bucket{le=\"%.4f\"} %d\n", bound, countBelow(sorted, int32(bound*1e6)))
	}
	fmt.Fprintf(&sb, "loadtest_latency_seconds_bucket{le=\"+Inf\"} %d\n", len(sorted))
	var sumUs int64
	for _, v := range sorted {
		sumUs += int64(v)
	}
	fmt.Fprintf(&sb, "loadtest_latency_seconds_sum %.6f\nloadtest_latency_seconds_count %d\n\n", float64(sumUs)/1e6, len(sorted))

	if logsSt != nil {
		ls := snapshotLogsSt(logsSt)
		fmt.Fprintf(&sb, "# HELP collect_logs_requests_total Requests to /.svc/collect_logs by status code\n")
		fmt.Fprintf(&sb, "# TYPE collect_logs_requests_total counter\n")
		for code, count := range ls.Codes {
			fmt.Fprintf(&sb, "collect_logs_requests_total{code=\"%s\"} %d\n", code, count)
		}
		if ls.Errors > 0 {
			fmt.Fprintf(&sb, "collect_logs_requests_total{code=\"error\"} %d\n", ls.Errors)
		}
		fmt.Fprintf(&sb, "\n")
		fmt.Fprintf(&sb, "# HELP collect_logs_latency_p95_seconds p95 latency for collect_logs calls\n")
		fmt.Fprintf(&sb, "# TYPE collect_logs_latency_p95_seconds gauge\ncollect_logs_latency_p95_seconds %.6f\n\n", ls.P95Ms/1000.0)
		fmt.Fprintf(&sb, "# HELP collect_logs_latency_p99_seconds p99 latency for collect_logs calls\n")
		fmt.Fprintf(&sb, "# TYPE collect_logs_latency_p99_seconds gauge\ncollect_logs_latency_p99_seconds %.6f\n\n", ls.P99Ms/1000.0)
	}

	w.Header().Set("Content-Type", "text/plain; version=0.0.4")
	_, _ = w.Write([]byte(sb.String()))
}
