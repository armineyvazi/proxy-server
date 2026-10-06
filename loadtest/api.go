package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"sync"
	"sync/atomic"
	"time"
)

// app is the singleton holding the state of the one in-flight test.
type app struct {
	mu          sync.Mutex
	state       string
	cfg         RunConfig
	start       time.Time
	cancel      context.CancelFunc
	st          *stats
	logsSt      *stats // separate store for /.svc/collect_logs interference calls
	hist        histBuf
	errs        errStore
	breakPt     *BreakInfo
	currentConc int64 // ramp mode: live worker count (atomic)
}

var g = &app{state: "idle"}

// BreakInfo records the load level at which a ramp test tripped its error threshold.
type BreakInfo struct {
	RPS    float64 `json:"rps"`
	ErrPct float64 `json:"err_pct"`
	AtSec  float64 `json:"at_s"`
}

// LogsSubStats mirrors the main stats for the collect_logs interference probe.
type LogsSubStats struct {
	Total  int64            `json:"total"`
	Errors int64            `json:"errors"`
	Codes  map[string]int64 `json:"codes"`
	P50Ms  float64          `json:"p50_ms"`
	P95Ms  float64          `json:"p95_ms"`
	P99Ms  float64          `json:"p99_ms"`
	AvgMs  float64          `json:"avg_ms"`
}

type StatusResp struct {
	State        string           `json:"state"`
	Config       *RunConfig       `json:"config,omitempty"`
	ElapsedSec   float64          `json:"elapsed_s"`
	Total        int64            `json:"total"`
	RPS          float64          `json:"rps"`
	Codes        map[string]int64 `json:"codes"`
	Errors       int64            `json:"errors"`
	P50Ms        float64          `json:"p50_ms"`
	P95Ms        float64          `json:"p95_ms"`
	P99Ms        float64          `json:"p99_ms"`
	MinMs        float64          `json:"min_ms"`
	MaxMs        float64          `json:"max_ms"`
	AvgMs        float64          `json:"avg_ms"`
	ErrPct       float64          `json:"err_pct"`
	LogsStats    *LogsSubStats    `json:"logs_stats,omitempty"`
	BreakPoint   *BreakInfo       `json:"break_point,omitempty"`
	RecentErrors []string         `json:"recent_errors,omitempty"`
	CurrentConc  int64            `json:"current_conc,omitempty"`
}

func handleRun(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	var cfg RunConfig
	if err := json.NewDecoder(r.Body).Decode(&cfg); err != nil {
		jsonError(w, "invalid JSON: "+err.Error(), 400)
		return
	}
	if cfg.URL == "" {
		jsonError(w, "url is required", 400)
		return
	}
	cfg.applyDefaults()

	g.mu.Lock()
	if g.state == "running" {
		g.mu.Unlock()
		jsonError(w, "test already running — POST /api/stop first", 409)
		return
	}
	dur := cfg.DurationSec
	if cfg.RampMode && dur < 3600 {
		dur = 3600 // ramp runs until the error threshold trips; cap at 1h
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(dur)*time.Second)
	g.state = "running"
	g.cfg = cfg
	g.start = time.Now()
	g.cancel = cancel
	g.st = newStats()
	if cfg.CollectLogsMs > 0 {
		g.logsSt = newStats()
	} else {
		g.logsSt = nil
	}
	g.hist.reset()
	g.errs = errStore{}
	g.breakPt = nil
	atomic.StoreInt64(&g.currentConc, 0)
	g.mu.Unlock()

	go runTest(ctx, cfg)

	writeJSON(w, map[string]string{"state": "running"})
}

func handleTargets(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, serverTargets)
}

func handleStop(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	g.mu.Lock()
	if g.cancel != nil {
		g.cancel()
	}
	g.mu.Unlock()
	writeJSON(w, map[string]string{"state": "stopping"})
}

// handleReset cancels any running test and clears all accumulated stats.
func handleReset(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	g.mu.Lock()
	if g.cancel != nil {
		g.cancel()
	}
	g.state = "idle"
	g.st = nil
	g.logsSt = nil
	g.hist.reset()
	g.errs = errStore{}
	g.breakPt = nil
	atomic.StoreInt64(&g.currentConc, 0)
	g.mu.Unlock()
	writeJSON(w, map[string]string{"state": "idle"})
}

func handleStatus(w http.ResponseWriter, r *http.Request) {
	g.mu.Lock()
	state := g.state
	cfg := g.cfg
	start := g.start
	st := g.st
	logsSt := g.logsSt
	breakPt := g.breakPt
	recentErrs := g.errs.recent()
	g.mu.Unlock()

	resp := StatusResp{State: state}
	if st != nil {
		codes, errors, total, sorted := st.snapshot()
		elapsed := time.Since(start).Seconds()
		resp.Config = &cfg
		resp.ElapsedSec = elapsed
		resp.Total = total
		resp.Codes = stringKeys(codes)
		resp.Errors = errors
		if elapsed > 0 {
			resp.RPS = float64(total) / elapsed
		}
		if len(sorted) > 0 {
			resp.P50Ms = percentile(sorted, 50)
			resp.P95Ms = percentile(sorted, 95)
			resp.P99Ms = percentile(sorted, 99)
			resp.MinMs = float64(sorted[0]) / 1000.0
			resp.MaxMs = float64(sorted[len(sorted)-1]) / 1000.0
			resp.AvgMs = meanMs(sorted)
		}
		if total > 0 {
			resp.ErrPct = float64(errors) / float64(total) * 100
		}
	}
	if logsSt != nil {
		resp.LogsStats = snapshotLogsSt(logsSt)
	}
	if breakPt != nil {
		resp.BreakPoint = breakPt
	}
	if len(recentErrs) > 0 {
		resp.RecentErrors = recentErrs
	}
	if cfg.RampMode {
		resp.CurrentConc = atomic.LoadInt64(&g.currentConc)
	}
	writeJSON(w, resp)
}

func handleHistory(w http.ResponseWriter, r *http.Request) {
	pts := g.hist.slice()
	w.Header().Set("Content-Type", "application/json")
	if pts == nil {
		_, _ = w.Write([]byte("[]"))
		return
	}
	_ = json.NewEncoder(w).Encode(pts)
}

func snapshotLogsSt(st *stats) *LogsSubStats {
	codes, errors, total, sorted := st.snapshot()
	ls := &LogsSubStats{Total: total, Errors: errors, Codes: stringKeys(codes)}
	if len(sorted) > 0 {
		ls.P50Ms = percentile(sorted, 50)
		ls.P95Ms = percentile(sorted, 95)
		ls.P99Ms = percentile(sorted, 99)
		ls.AvgMs = meanMs(sorted)
	}
	return ls
}

func stringKeys(codes map[int]int64) map[string]int64 {
	out := make(map[string]int64, len(codes))
	for k, v := range codes {
		out[fmt.Sprintf("%d", k)] = v
	}
	return out
}

func meanMs(sorted []int32) float64 {
	var sum int64
	for _, v := range sorted {
		sum += int64(v)
	}
	return float64(sum) / float64(len(sorted)) / 1000.0
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}

func jsonError(w http.ResponseWriter, msg string, code int) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(map[string]string{"error": msg})
}
