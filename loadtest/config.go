package main

import "os"

// TargetPreset is a one-click target shown in the UI, injected by docker-compose.
type TargetPreset struct {
	Name string `json:"name"`
	URL  string `json:"url"`
	Host string `json:"host"`
}

var serverTargets []TargetPreset

func initTargets() {
	if u := os.Getenv("PROXY_URL"); u != "" {
		serverTargets = append(serverTargets, TargetPreset{
			Name: "rust-proxy-server",
			URL:  u,
			Host: os.Getenv("PROXY_HOST"),
		})
	}
	if u := os.Getenv("NGINX_URL"); u != "" {
		serverTargets = append(serverTargets, TargetPreset{
			Name: "nginx",
			URL:  u,
			Host: os.Getenv("NGINX_HOST"),
		})
	}
}

// RunConfig is a single load-test configuration sent by the UI.
// Fields are ordered largest-alignment-first (strings, float, ints, bools) to
// avoid padding; JSON tags are unaffected by ordering.
type RunConfig struct {
	URL           string `json:"url"`
	Host          string `json:"host"`
	Method        string `json:"method"`
	ExtraHeaders  string `json:"extra_headers"`
	ScenarioLabel string `json:"scenario"`

	RampStopErrPct float64 `json:"ramp_stop_err_pct"`

	Concurrency   int `json:"concurrency"`
	DurationSec   int `json:"duration_s"`
	MaxRPS        int `json:"max_rps"`
	RampUpSec     int `json:"ramp_up_s"`
	TimeoutSec    int `json:"timeout_s"`
	CollectLogsMs int `json:"collect_logs_ms"`
	SLAMs         int `json:"sla_ms"`
	RampStartConc int `json:"ramp_start_conc"`
	RampEndConc   int `json:"ramp_end_conc"`
	RampStepSize  int `json:"ramp_step_size"`
	RampStepSec   int `json:"ramp_step_s"`

	NoKeepalive   bool `json:"no_keepalive"`
	RampMode      bool `json:"ramp_mode"`
	MixedMethods  bool `json:"mixed_methods"`
	LoggingImpact bool `json:"logging_impact"`
}

func (c *RunConfig) applyDefaults() {
	if c.Concurrency <= 0 {
		c.Concurrency = 50
	}
	if c.DurationSec <= 0 {
		c.DurationSec = 30
	}
	if c.TimeoutSec <= 0 {
		c.TimeoutSec = 30
	}
	if c.Method == "" {
		c.Method = "GET"
	}
	if c.RampMode {
		if c.RampStartConc <= 0 {
			c.RampStartConc = 10
		}
		if c.RampEndConc <= 0 {
			c.RampEndConc = 2000
		}
		if c.RampStepSize <= 0 {
			c.RampStepSize = 50
		}
		if c.RampStepSec <= 0 {
			c.RampStepSec = 10
		}
		if c.RampStopErrPct <= 0 {
			c.RampStopErrPct = 10.0
		}
		if c.DurationSec <= 30 {
			c.DurationSec = 3600
		}
	}
	if c.LoggingImpact && c.CollectLogsMs <= 0 {
		c.CollectLogsMs = 200
	}
	if c.ScenarioLabel == "" {
		c.ScenarioLabel = "default"
		if c.RampMode {
			c.ScenarioLabel = "load-ramp"
		}
		if c.LoggingImpact {
			c.ScenarioLabel = "logging-impact"
		}
		if c.MixedMethods {
			c.ScenarioLabel = "mixed-methods"
		}
	}
}
