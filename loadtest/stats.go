package main

import (
	"sort"
	"sync"
)

const (
	maxSamplesTotal = 1_000_000
	nShards         = 32 // shard the latency store to cut lock contention under high concurrency
	maxHistory      = 600
	maxErrSamples   = 30
)

// stats accumulates request outcomes across nShards independent locks. Each
// worker is pinned to one shard by workerID%nShards, so contention scales with
// workers/nShards rather than workers.
type stats struct {
	shards      [nShards]statShard
	capPerShard int
}

type statShard struct {
	mu      sync.Mutex
	codes   map[int]int64
	errors  int64
	samples []int32 // latency µs
	total   int64
}

func newStats() *stats {
	s := &stats{capPerShard: maxSamplesTotal / nShards}
	for i := range s.shards {
		s.shards[i].codes = make(map[int]int64)
		s.shards[i].samples = make([]int32, 0, s.capPerShard)
	}
	return s
}

func (s *stats) record(shard int, latencyUs int32, code int, isErr bool) {
	sh := &s.shards[shard]
	sh.mu.Lock()
	sh.total++
	if isErr {
		sh.errors++
	} else {
		sh.codes[code]++
		if len(sh.samples) < s.capPerShard {
			sh.samples = append(sh.samples, latencyUs)
		}
	}
	sh.mu.Unlock()
}

// snapshot merges all shards into one view with a sorted latency slice.
func (s *stats) snapshot() (codes map[int]int64, errors, total int64, sorted []int32) {
	codes = make(map[int]int64)
	for i := range s.shards {
		sh := &s.shards[i]
		sh.mu.Lock()
		for k, v := range sh.codes {
			codes[k] += v
		}
		errors += sh.errors
		total += sh.total
		sorted = append(sorted, sh.samples...)
		sh.mu.Unlock()
	}
	sort.Slice(sorted, func(i, j int) bool { return sorted[i] < sorted[j] })
	return
}

// percentile returns the p-th percentile of sorted µs latencies, in ms.
func percentile(sorted []int32, p float64) float64 {
	if len(sorted) == 0 {
		return 0
	}
	idx := int(float64(len(sorted))*p/100.0 + 0.5)
	if idx >= len(sorted) {
		idx = len(sorted) - 1
	}
	return float64(sorted[idx]) / 1000.0
}

func countBelow(sorted []int32, thresholdUs int32) int {
	lo, hi := 0, len(sorted)
	for lo < hi {
		mid := (lo + hi) / 2
		if sorted[mid] <= thresholdUs {
			lo = mid + 1
		} else {
			hi = mid
		}
	}
	return lo
}

// HistPoint is a one-second snapshot for the UI time-series charts.
type HistPoint struct {
	T      int64   `json:"t"` // Unix ms
	RPS    float64 `json:"rps"`
	P50Ms  float64 `json:"p50"`
	P95Ms  float64 `json:"p95"`
	P99Ms  float64 `json:"p99"`
	ErrPct float64 `json:"ep"`
}

type histBuf struct {
	mu    sync.Mutex
	buf   [maxHistory]HistPoint
	head  int
	count int
}

func (h *histBuf) reset() {
	h.mu.Lock()
	h.head = 0
	h.count = 0
	h.mu.Unlock()
}

func (h *histBuf) push(p HistPoint) {
	h.mu.Lock()
	h.buf[h.head] = p
	h.head = (h.head + 1) % maxHistory
	if h.count < maxHistory {
		h.count++
	}
	h.mu.Unlock()
}

func (h *histBuf) slice() []HistPoint {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.count == 0 {
		return nil
	}
	out := make([]HistPoint, h.count)
	start := (h.head - h.count + maxHistory) % maxHistory
	for i := 0; i < h.count; i++ {
		out[i] = h.buf[(start+i)%maxHistory]
	}
	return out
}

// errStore keeps the most recent error messages for display in the UI.
type errStore struct {
	mu   sync.Mutex
	msgs [maxErrSamples]string
	pos  int
}

func (e *errStore) add(msg string) {
	e.mu.Lock()
	e.msgs[e.pos%maxErrSamples] = msg
	e.pos++
	e.mu.Unlock()
}

func (e *errStore) recent() []string {
	e.mu.Lock()
	defer e.mu.Unlock()
	n := e.pos
	if n > maxErrSamples {
		n = maxErrSamples
	}
	out := make([]string, n)
	start := e.pos - n
	for i := 0; i < n; i++ {
		out[i] = e.msgs[(start+i)%maxErrSamples]
	}
	return out
}
