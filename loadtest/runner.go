package main

import (
	"context"
	"fmt"
	"io"
	"math/rand"
	"net"
	"net/http"
	"net/url"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

// mixedMethods is the rotation used when MixedMethods is set: mostly GET, some
// POST, the occasional HEAD — a rough stand-in for real traffic.
var mixedMethods = [10]string{"GET", "GET", "GET", "GET", "GET", "POST", "POST", "POST", "HEAD", "GET"}

func runTest(ctx context.Context, cfg RunConfig) {
	timeout := time.Duration(cfg.TimeoutSec) * time.Second
	extraHeaders := parseHeaders(cfg.ExtraHeaders)
	dialer := &net.Dialer{Timeout: 10 * time.Second, KeepAlive: 30 * time.Second}
	client := newHTTPClient(cfg, dialer, timeout)
	rateLimiter := newRateLimiter(ctx, cfg.MaxRPS)

	go sampleHistory(ctx)
	if cfg.CollectLogsMs > 0 {
		startCollectLogsProbe(ctx, cfg, dialer, timeout)
	}

	var wg sync.WaitGroup
	launch := func(workerIdx int) {
		wg.Add(1)
		go func() {
			defer wg.Done()
			shard := workerIdx % nShards
			for ctx.Err() == nil {
				if rateLimiter != nil {
					select {
					case <-rateLimiter:
					case <-ctx.Done():
						return
					}
				}
				doRequest(ctx, client, cfg, extraHeaders, g.st, shard)
			}
		}()
	}

	if cfg.RampMode {
		runRamp(ctx, cfg, launch)
	} else {
		runFixed(ctx, cfg, launch)
	}

	wg.Wait()
	g.mu.Lock()
	g.state = "done"
	g.mu.Unlock()
}

func newHTTPClient(cfg RunConfig, dialer *net.Dialer, timeout time.Duration) *http.Client {
	return &http.Client{
		Transport: &http.Transport{
			DialContext:         dialer.DialContext,
			MaxIdleConnsPerHost: cfg.Concurrency + 64,
			MaxConnsPerHost:     cfg.Concurrency + 64,
			IdleConnTimeout:     60 * time.Second,
			DisableKeepAlives:   cfg.NoKeepalive,
			DisableCompression:  true,
		},
		Timeout:       timeout,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
}

// runFixed launches a constant worker pool, optionally spreading worker starts
// over RampUpSec as a soft warm-up.
func runFixed(ctx context.Context, cfg RunConfig, launch func(int)) {
	for i := 0; i < cfg.Concurrency; i++ {
		idx := i
		if cfg.RampUpSec <= 0 {
			launch(idx)
			continue
		}
		delay := time.Duration(idx) * time.Second * time.Duration(cfg.RampUpSec) / time.Duration(cfg.Concurrency)
		go func() {
			select {
			case <-time.After(delay):
				if ctx.Err() == nil {
					launch(idx)
				}
			case <-ctx.Done():
			}
		}()
	}
	atomic.StoreInt64(&g.currentConc, int64(cfg.Concurrency))
}

// runRamp starts RampStartConc workers and adds RampStepSize more every
// RampStepSec until RampEndConc, finding the breaking point gradually. A second
// goroutine cancels the test once the error rate crosses RampStopErrPct.
func runRamp(ctx context.Context, cfg RunConfig, launch func(int)) {
	for i := 0; i < cfg.RampStartConc; i++ {
		launch(i)
	}
	atomic.StoreInt64(&g.currentConc, int64(cfg.RampStartConc))

	go func() {
		conc := cfg.RampStartConc
		ticker := time.NewTicker(time.Duration(cfg.RampStepSec) * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				toAdd := min(cfg.RampStepSize, cfg.RampEndConc-conc)
				if toAdd <= 0 {
					return
				}
				for i := conc; i < conc+toAdd; i++ {
					launch(i)
				}
				conc += toAdd
				atomic.StoreInt64(&g.currentConc, int64(conc))
				eprintln("ramp: concurrency now %d", conc)
			}
		}
	}()

	go watchErrorThreshold(ctx, cfg.RampStopErrPct)
}

// watchErrorThreshold cancels the run when the error rate exceeds threshold
// percent (after a minimum sample size), recording the break point.
func watchErrorThreshold(ctx context.Context, threshold float64) {
	if threshold <= 0 {
		threshold = 10.0
	}
	ticker := time.NewTicker(3 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			g.mu.Lock()
			st := g.st
			start := g.start
			g.mu.Unlock()
			if st == nil {
				continue
			}
			_, errs, total, _ := st.snapshot()
			if total < 200 {
				continue
			}
			errPct := float64(errs) / float64(total) * 100
			if errPct < threshold {
				continue
			}
			elapsed := time.Since(start).Seconds()
			rps := float64(total) / elapsed
			conc := atomic.LoadInt64(&g.currentConc)
			g.mu.Lock()
			g.breakPt = &BreakInfo{RPS: rps, ErrPct: errPct, AtSec: elapsed}
			if g.cancel != nil {
				g.cancel()
			}
			g.mu.Unlock()
			eprintln("ramp stop: %.1f%% errors at %.0f RPS (conc=%d)", errPct, rps, conc)
			return
		}
	}
}

// sampleHistory pushes a one-second snapshot of instantaneous RPS and latency
// percentiles into the history ring for the UI charts.
func sampleHistory(ctx context.Context) {
	var prevTotal int64
	prevTime := time.Now()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-ticker.C:
			g.mu.Lock()
			st := g.st
			g.mu.Unlock()
			if st == nil {
				continue
			}
			_, errs, total, sorted := st.snapshot()
			dt := now.Sub(prevTime).Seconds()
			instRPS := 0.0
			if dt > 0 {
				instRPS = float64(total-prevTotal) / dt
			}
			prevTotal, prevTime = total, now
			ep := 0.0
			if total > 0 {
				ep = float64(errs) / float64(total) * 100
			}
			g.hist.push(HistPoint{
				T:      now.UnixMilli(),
				RPS:    instRPS,
				P50Ms:  percentile(sorted, 50),
				P95Ms:  percentile(sorted, 95),
				P99Ms:  percentile(sorted, 99),
				ErrPct: ep,
			})
		}
	}
}

// startCollectLogsProbe fires GET /.svc/collect_logs on its own small connection
// pool so the interference measurement never competes with the main workers.
func startCollectLogsProbe(ctx context.Context, cfg RunConfig, dialer *net.Dialer, timeout time.Duration) {
	logsURL, err := buildCollectLogsURL(cfg.URL)
	if err != nil {
		return
	}
	logsCfg := RunConfig{URL: logsURL, Host: cfg.Host, Method: "GET", TimeoutSec: cfg.TimeoutSec}
	client := &http.Client{
		Transport: &http.Transport{
			DialContext:         dialer.DialContext,
			MaxIdleConnsPerHost: 4,
			MaxConnsPerHost:     4,
			IdleConnTimeout:     60 * time.Second,
			DisableCompression:  true,
		},
		Timeout:       timeout,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	go func() {
		ticker := time.NewTicker(time.Duration(cfg.CollectLogsMs) * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				g.mu.Lock()
				st := g.logsSt
				g.mu.Unlock()
				if st != nil {
					doRequest(ctx, client, logsCfg, nil, st, 0)
				}
			}
		}
	}()
}

func buildCollectLogsURL(baseURL string) (string, error) {
	u, err := url.Parse(baseURL)
	if err != nil {
		return "", err
	}
	u.Path = "/.svc/collect_logs"
	u.RawQuery = ""
	return u.String(), nil
}

// doRequest fires one request and records latency + status into st. POST is
// non-idempotent and Go won't retry it on a stale pooled connection, so we force
// Close=true for POST and retry once on idle-connection errors for other methods.
func doRequest(ctx context.Context, client *http.Client, cfg RunConfig, extraHeaders map[string]string, st *stats, shard int) {
	method := cfg.Method
	if cfg.MixedMethods {
		method = mixedMethods[rand.Intn(len(mixedMethods))]
	}

	const maxAttempts = 2
	var (
		resp  *http.Response
		err   error
		latUs int32
	)
	for attempt := 0; attempt < maxAttempts; attempt++ {
		req, reqErr := http.NewRequestWithContext(ctx, method, cfg.URL, nil)
		if reqErr != nil {
			return
		}
		if cfg.Host != "" {
			req.Host = cfg.Host
		}
		for k, v := range extraHeaders {
			req.Header.Set(k, v)
		}
		if method == "POST" {
			req.Close = true
		}

		t0 := time.Now()
		resp, err = client.Do(req)
		latUs = int32(time.Since(t0).Microseconds())

		if err == nil || ctx.Err() != nil {
			break
		}
		if attempt < maxAttempts-1 && isIdleConnError(err) {
			continue
		}
		break
	}

	if err != nil {
		if ctx.Err() == nil {
			st.record(shard, latUs, 0, true)
			g.errs.add(categorizeErr(err))
		}
		return
	}
	_, _ = io.Copy(io.Discard, resp.Body) // drain so the connection can be reused
	_ = resp.Body.Close()
	st.record(shard, latUs, resp.StatusCode, false)
}

// isIdleConnError reports whether err comes from reusing a connection the server
// already closed — safe to retry once.
func isIdleConnError(err error) bool {
	s := err.Error()
	return strings.Contains(s, "server closed idle connection") ||
		strings.Contains(s, "use of closed network connection") ||
		strings.Contains(s, "connection reset by peer")
}

func categorizeErr(err error) string {
	s := err.Error()
	switch {
	case strings.Contains(s, "connection refused"):
		return "connection refused"
	case strings.Contains(s, "timeout") || strings.Contains(s, "deadline exceeded"):
		return "timeout"
	case strings.Contains(s, "server closed idle connection"):
		return "server closed idle connection"
	case strings.Contains(s, "EOF") || strings.Contains(s, "connection reset"):
		return "connection reset (EOF)"
	case strings.Contains(s, "no such host"):
		return "DNS failure"
	case strings.Contains(s, "too many open files"):
		return "too many open files (raise ulimit)"
	default:
		if len(s) > 80 {
			s = s[:80] + "…"
		}
		return s
	}
}

// newRateLimiter returns a token channel releasing maxRPS tokens/second, or nil
// for unlimited. Sub-1000 RPS uses a coarser tick; above that we batch per ms.
func newRateLimiter(ctx context.Context, maxRPS int) <-chan struct{} {
	if maxRPS <= 0 {
		return nil
	}
	buf := min(maxRPS, 10000)
	tokens := make(chan struct{}, buf)
	go func() {
		tickRate := time.Millisecond
		perTick := maxRPS / 1000
		if perTick < 1 {
			perTick = 1
			tickRate = time.Duration(float64(time.Second) / float64(maxRPS))
		}
		ticker := time.NewTicker(tickRate)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				for i := 0; i < perTick; i++ {
					select {
					case tokens <- struct{}{}:
					default:
					}
				}
			}
		}
	}()
	return tokens
}

func parseHeaders(raw string) map[string]string {
	h := make(map[string]string)
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if parts := strings.SplitN(line, ":", 2); len(parts) == 2 {
			h[strings.TrimSpace(parts[0])] = strings.TrimSpace(parts[1])
		}
	}
	return h
}

func eprintln(format string, args ...any) {
	fmt.Fprintf(os.Stderr, format+"\n", args...)
}
