package main

import "net/http"

func handleUI(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = w.Write([]byte(uiHTML))
}

const uiHTML = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Proxy Load Tester</title>
<style>
:root{--bg:#0f1117;--card:#1a1f2e;--card2:#222736;--border:#2d3448;--text:#e8eaf0;
--muted:#7b82a0;--accent:#4f8ef7;--green:#22c55e;--yellow:#f59e0b;--red:#ef4444;--purple:#a855f7}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--text);font-family:'Inter','Segoe UI',system-ui,sans-serif;min-height:100vh;display:flex;flex-direction:column;font-size:14px}
header{background:var(--card);border-bottom:1px solid var(--border);padding:12px 20px;display:flex;align-items:center;gap:10px}
header h1{font-size:16px;font-weight:600;letter-spacing:-.01em}
.badge{font-size:10px;padding:2px 8px;border-radius:9999px;background:var(--accent);color:#fff;font-weight:600}
.main{flex:1;display:grid;grid-template-columns:360px 1fr;min-height:0}
/* ── Sidebar ── */
.sidebar{background:var(--card);border-right:1px solid var(--border);padding:18px;display:flex;flex-direction:column;gap:14px;overflow-y:auto}
.presets{display:grid;grid-template-columns:repeat(5,1fr);gap:5px}
.preset-btn{padding:7px 4px;border:1px solid var(--border);border-radius:6px;background:var(--card2);color:var(--muted);font-size:11px;font-weight:600;cursor:pointer;text-align:center;transition:all .15s;line-height:1.3}
.preset-btn:hover{border-color:var(--accent);color:var(--text)}
.preset-btn.active{border-color:var(--accent);background:#1a2744;color:var(--accent)}
.preset-name{display:block;font-size:10px;text-transform:uppercase;letter-spacing:.04em;margin-bottom:2px}
.preset-sub{display:block;font-size:10px;color:var(--muted);font-weight:400}
.target-grid{display:grid;grid-template-columns:1fr 1fr;gap:8px}
.target-card{padding:12px 10px;border:2px solid var(--border);border-radius:8px;background:var(--card2);
  cursor:pointer;transition:all .15s;text-align:center;user-select:none}
.target-card:hover{border-color:var(--accent)}
.target-card.active{border-color:var(--accent);background:#1a2744}
.target-card .tc-icon{font-size:22px;margin-bottom:4px}
.target-card .tc-name{font-size:12px;font-weight:700;color:var(--text);display:block}
.target-card .tc-url{font-size:10px;color:var(--muted);display:block;margin-top:2px;
  overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.divider{height:1px;background:var(--border);margin:2px 0}
.field label{display:block;font-size:11px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;margin-bottom:5px}
.field input[type=text],.field input[type=number],.field select,.field textarea{
  width:100%;background:var(--bg);border:1px solid var(--border);color:var(--text);
  padding:7px 10px;border-radius:6px;font-size:13px;font-family:inherit}
.field input:focus,.field select:focus,.field textarea:focus{outline:none;border-color:var(--accent)}
.field textarea{min-height:64px;resize:vertical;font-family:'SF Mono','Fira Code',monospace;font-size:12px}
.conc-row{display:flex;align-items:center;gap:6px}
.conc-row input[type=number]{flex:1}
.chips{display:flex;gap:4px;flex-wrap:wrap;margin-top:5px}
.chip{padding:3px 8px;border:1px solid var(--border);border-radius:4px;background:var(--card2);
  color:var(--muted);font-size:11px;font-weight:600;cursor:pointer;transition:all .12s}
.chip:hover{border-color:var(--accent);color:var(--text)}
.toggle-row{display:flex;align-items:center;gap:8px}
.toggle-row input{width:15px;height:15px;accent-color:var(--accent);flex-shrink:0}
.method-btns{display:flex;gap:5px}
.method-btn{padding:5px 12px;border:1px solid var(--border);border-radius:5px;
  background:var(--card2);color:var(--muted);font-size:12px;font-weight:600;cursor:pointer;transition:all .12s}
.method-btn.active{border-color:var(--accent);background:#1a2744;color:var(--accent)}
.adv-toggle{display:flex;align-items:center;gap:6px;cursor:pointer;color:var(--muted);font-size:12px;font-weight:600;user-select:none}
.adv-toggle:hover{color:var(--text)}
.adv-body{display:flex;flex-direction:column;gap:12px;padding-top:4px}
.btn{padding:11px 20px;border:none;border-radius:7px;font-size:13px;font-weight:600;cursor:pointer;transition:opacity .15s;width:100%;letter-spacing:.01em}
.btn:hover{opacity:.85}
.btn-start{background:var(--green);color:#fff}
.btn-stop{background:var(--red);color:#fff;display:none}
.btn-clear{background:var(--card2);color:var(--muted);border:1px solid var(--border)}
.btn-clear:hover{border-color:var(--purple);color:var(--purple)}
/* ── Results ── */
.results{padding:20px;display:flex;flex-direction:column;gap:16px;overflow-y:auto}
.idle-hint{display:flex;flex-direction:column;align-items:center;justify-content:center;flex:1;gap:12px;color:var(--muted);text-align:center;padding:40px}
.idle-hint svg{opacity:.2}
.state-bar{display:flex;align-items:center;gap:10px;font-size:14px;font-weight:600}
.dot{width:9px;height:9px;border-radius:50%;background:var(--muted);flex-shrink:0}
.dot.running{background:var(--green);animation:pulse 1.2s infinite}
.dot.done{background:var(--accent)}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.2}}
.cards{display:grid;grid-template-columns:repeat(3,1fr);gap:10px}
.card{background:var(--card);border:1px solid var(--border);border-radius:8px;padding:14px}
.card .lbl{font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.06em;color:var(--muted);margin-bottom:6px}
.card .val{font-size:24px;font-weight:700;line-height:1;letter-spacing:-.01em}
.section{background:var(--card);border:1px solid var(--border);border-radius:8px;overflow:hidden}
.section-hdr{padding:10px 14px;font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.06em;color:var(--muted);border-bottom:1px solid var(--border);background:var(--card2)}
table{width:100%;border-collapse:collapse}
th{text-align:left;padding:7px 14px;font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.05em;color:var(--muted);border-bottom:1px solid var(--border);background:var(--card2)}
td{padding:9px 14px;border-bottom:1px solid var(--border);font-variant-numeric:tabular-nums}
tr:last-child td{border-bottom:none}
.bar-wrap{background:var(--border);height:5px;border-radius:3px;overflow:hidden;min-width:60px}
.bar-fill{height:100%;border-radius:3px;transition:width .35s}
.code-row{display:flex;align-items:center;gap:10px;padding:9px 14px;border-bottom:1px solid var(--border)}
.code-row:last-child{border-bottom:none}
.code-lbl{font-size:12px;font-weight:700;min-width:48px;font-variant-numeric:tabular-nums}
.code-bar{flex:1;height:7px;border-radius:4px;transition:width .35s}
.code-cnt{font-size:12px;color:var(--muted);min-width:110px;text-align:right}
.c2{background:var(--green)}.c4{background:var(--yellow)}.c5{background:var(--red)}.cerr{background:#6b7280}
.info-grid{display:grid;grid-template-columns:repeat(3,1fr);gap:10px}
.info-card{background:var(--card2);border:1px solid var(--border);border-radius:6px;padding:10px 14px;font-size:12px}
.info-card .k{color:var(--muted);margin-bottom:2px;font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.05em}
.info-card .v{font-weight:600;font-variant-numeric:tabular-nums}
.chart-box{padding:4px 14px 10px}
.err-list{padding:10px 14px;font-family:'SF Mono','Fira Code',monospace;font-size:11px;color:var(--red);display:flex;flex-direction:column;gap:3px}
.err-item{padding:3px 8px;background:#450a0a33;border-radius:4px;border-left:2px solid var(--red)}
.break-box{padding:16px;display:flex;flex-direction:column;gap:10px}
.break-stat{display:flex;gap:8px;align-items:baseline}
.break-stat .bk{font-size:11px;color:var(--muted);font-weight:700;text-transform:uppercase;width:120px}
.break-stat .bv{font-size:22px;font-weight:800;font-variant-numeric:tabular-nums}
.errpct-bar{height:6px;border-radius:3px;margin-top:4px;background:var(--border);overflow:hidden}
.errpct-fill{height:100%;border-radius:3px;background:var(--red);transition:width .5s}
</style>
</head>
<body>
<header>
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="#4f8ef7" stroke-width="2.2">
    <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
  </svg>
  <h1>Proxy Load Tester</h1>
  <span class="badge">HTTP/1.1</span>
</header>

<div class="main">
<div class="sidebar">

  <!-- Target selector (populated from /api/targets) -->
  <div class="field" id="target-section" style="display:none">
    <label>Select Target</label>
    <div class="target-grid" id="target-grid"></div>
  </div>

  <!-- Load presets -->
  <div class="field">
    <label>Load Profile Presets</label>
    <div class="presets">
      <button class="preset-btn" onclick="applyPreset('warmup')">
        <span class="preset-name">Warmup</span>
        <span class="preset-sub">10c / 30s</span>
      </button>
      <button class="preset-btn" onclick="applyPreset('baseline')">
        <span class="preset-name">Baseline</span>
        <span class="preset-sub">100c / 60s</span>
      </button>
      <button class="preset-btn" onclick="applyPreset('load')">
        <span class="preset-name">Load</span>
        <span class="preset-sub">500c / 60s</span>
      </button>
      <button class="preset-btn" onclick="applyPreset('stress')">
        <span class="preset-name">Stress</span>
        <span class="preset-sub">2000c / 30s</span>
      </button>
      <button class="preset-btn" onclick="applyPreset('spike')">
        <span class="preset-name">Spike</span>
        <span class="preset-sub">5000c / 10s</span>
      </button>
      <button class="preset-btn" onclick="applyPreset('ramp')" style="border-color:#6366f1">
        <span class="preset-name" style="color:#6366f1">&#x1F4C8; Ramp</span>
        <span class="preset-sub">10→2000c</span>
      </button>
    </div>
  </div>

  <div class="divider"></div>

  <!-- Target -->
  <div class="field">
    <label>Target URL</label>
    <input id="url" type="text" value="http://proxy-server:8080/" placeholder="http://...">
  </div>
  <div class="field">
    <label>Host Header <span style="text-transform:none;font-weight:400;font-size:10px">(proxy-server validates this)</span></label>
    <input id="host" type="text" value="upstream" placeholder="upstream">
  </div>

  <!-- Concurrency -->
  <div class="field">
    <label>Concurrency (workers)</label>
    <div class="conc-row">
      <input id="conc" type="number" min="1" max="20000" value="50" style="font-size:16px;font-weight:700">
    </div>
    <div class="chips" style="margin-top:6px">
      <span class="chip" onclick="setConc(10)">10</span>
      <span class="chip" onclick="setConc(50)">50</span>
      <span class="chip" onclick="setConc(100)">100</span>
      <span class="chip" onclick="setConc(200)">200</span>
      <span class="chip" onclick="setConc(500)">500</span>
      <span class="chip" onclick="setConc(1000)">1k</span>
      <span class="chip" onclick="setConc(2000)">2k</span>
      <span class="chip" onclick="setConc(5000)">5k</span>
    </div>
  </div>

  <!-- Duration -->
  <div class="field">
    <label>Duration</label>
    <select id="dur">
      <option value="10">10 seconds</option>
      <option value="30" selected>30 seconds</option>
      <option value="60">1 minute</option>
      <option value="120">2 minutes</option>
      <option value="300">5 minutes</option>
      <option value="600">10 minutes</option>
    </select>
  </div>

  <div class="divider"></div>

  <!-- Advanced options (collapsible) -->
  <div>
    <div class="adv-toggle" onclick="toggleAdv()" id="adv-toggle">
      <span id="adv-arrow" style="font-size:10px">▶</span>
      Advanced Options
    </div>
    <div class="adv-body" id="adv-body" style="display:none;margin-top:12px">

      <div class="field">
        <label>HTTP Method</label>
        <div class="method-btns">
          <button class="method-btn active" id="m-GET"  onclick="setMethod('GET')">GET</button>
          <button class="method-btn"        id="m-POST" onclick="setMethod('POST')">POST</button>
          <button class="method-btn"        id="m-PUT"  onclick="setMethod('PUT')">PUT</button>
          <button class="method-btn"        id="m-HEAD" onclick="setMethod('HEAD')">HEAD</button>
        </div>
      </div>

      <div class="field">
        <label>Max RPS <span style="text-transform:none;font-weight:400;font-size:10px">(0 = full throttle)</span></label>
        <input id="maxrps" type="number" min="0" max="1000000" value="0" placeholder="0">
        <div style="font-size:11px;color:var(--muted);margin-top:4px">Rate-limit requests per second across all workers</div>
      </div>

      <div class="field">
        <label>Ramp-up Period</label>
        <select id="rampup">
          <option value="0" selected>Instant (all workers start at once)</option>
          <option value="5">5 seconds</option>
          <option value="10">10 seconds</option>
          <option value="30">30 seconds</option>
          <option value="60">60 seconds</option>
        </select>
      </div>

      <div class="field">
        <label>Request Timeout</label>
        <select id="timeout">
          <option value="5">5 seconds</option>
          <option value="10">10 seconds</option>
          <option value="30" selected>30 seconds</option>
          <option value="60">60 seconds</option>
        </select>
      </div>

      <div class="field">
        <div class="toggle-row">
          <input id="nkv" type="checkbox">
          <label for="nkv" style="text-transform:none;font-size:13px;font-weight:400">
            Disable keep-alive (new TCP connection per request)
          </label>
        </div>
      </div>

      <div class="field">
        <label>SLA Target <span style="text-transform:none;font-weight:400;font-size:10px">(p99 threshold, ms)</span></label>
        <input id="sla-ms" type="number" min="1" max="60000" value="100" placeholder="100">
        <div style="font-size:11px;color:var(--muted);margin-top:4px">Red dashed line on latency chart. Values above = SLA breach.</div>
      </div>

      <!-- Load-ramp mode -->
      <div class="field">
        <div class="toggle-row">
          <input id="ramp-mode" type="checkbox" onchange="toggleRampMode()">
          <label for="ramp-mode" style="text-transform:none;font-size:13px;font-weight:400">
            &#x1F4C8; Load-ramp mode <span style="color:var(--muted);font-size:11px">(gradually increase concurrency)</span>
          </label>
        </div>
        <div id="ramp-opts" style="display:none;flex-direction:column;gap:6px;margin-top:8px">
          <div style="display:grid;grid-template-columns:1fr 1fr;gap:6px">
            <div>
              <label style="display:block;font-size:11px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;margin-bottom:3px">Start workers</label>
              <input id="ramp-start" type="number" min="1" max="10000" value="10" style="width:100%">
            </div>
            <div>
              <label style="display:block;font-size:11px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;margin-bottom:3px">Max workers</label>
              <input id="ramp-end" type="number" min="1" max="20000" value="2000" style="width:100%">
            </div>
            <div>
              <label style="display:block;font-size:11px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;margin-bottom:3px">Step size</label>
              <input id="ramp-step" type="number" min="1" max="1000" value="50" style="width:100%">
            </div>
            <div>
              <label style="display:block;font-size:11px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;margin-bottom:3px">Step (sec)</label>
              <input id="ramp-step-s" type="number" min="1" max="300" value="10" style="width:100%">
            </div>
          </div>
          <div>
            <label style="display:block;font-size:11px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;margin-bottom:3px">Auto-stop error% threshold</label>
            <input id="ramp-err-pct" type="number" min="0.1" max="100" step="0.5" value="10" style="width:100px">
            <span style="font-size:11px;color:var(--muted);margin-left:6px">% (0 = never auto-stop)</span>
          </div>
          <div style="font-size:11px;color:var(--muted)">Ramp adds <i>step&nbsp;size</i> workers every <i>step&nbsp;sec</i> until max or error threshold.</div>
        </div>
      </div>

      <!-- Mixed HTTP methods -->
      <div class="field">
        <div class="toggle-row">
          <input id="mixed-methods" type="checkbox">
          <label for="mixed-methods" style="text-transform:none;font-size:13px;font-weight:400">
            Mixed HTTP methods <span style="color:var(--muted);font-size:11px">(50% GET / 30% POST / 20% HEAD)</span>
          </label>
        </div>
      </div>

      <!-- Logging impact test -->
      <div class="field">
        <div class="toggle-row">
          <input id="logging-impact" type="checkbox" onchange="toggleLoggingImpact()">
          <label for="logging-impact" style="text-transform:none;font-size:13px;font-weight:400">
            Logging impact test <span style="color:var(--muted);font-size:11px">(poll collect_logs every 200ms)</span>
          </label>
        </div>
      </div>

      <div class="field">
        <div class="toggle-row">
          <input id="cl-enabled" type="checkbox" onchange="toggleClInterval()">
          <label for="cl-enabled" style="text-transform:none;font-size:13px;font-weight:400">
            collect_logs interference test
          </label>
        </div>
        <div id="cl-interval-row" style="display:none;margin-top:8px">
          <label style="display:block;font-size:11px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;margin-bottom:5px">Poll interval</label>
          <select id="cl-interval">
            <option value="100">every 100 ms &mdash; 10 calls/sec</option>
            <option value="200" selected>every 200 ms &mdash; 5 calls/sec</option>
            <option value="500">every 500 ms &mdash; 2 calls/sec</option>
            <option value="1000">every 1000 ms &mdash; 1 call/sec</option>
          </select>
          <div style="font-size:11px;color:var(--muted);margin-top:5px">
            Fires <code style="font-size:10px">/.svc/collect_logs</code> in parallel using a separate goroutine.
            Tracks its own p95/p99 so you can verify it does not interfere with proxy latency.
          </div>
        </div>
      </div>

      <div class="field">
        <label>Extra Headers <span style="text-transform:none;font-weight:400;font-size:10px">(one per line: Key: Value)</span></label>
        <textarea id="headers" placeholder="X-Custom-Header: value&#10;Accept: application/json"></textarea>
      </div>

    </div>
  </div>

  <div style="flex:1"></div>

  <button class="btn btn-start" id="btn-s" onclick="startTest()">&#9654;&#160; Start Test</button>
  <button class="btn btn-stop"  id="btn-x" onclick="stopTest()">&#9632;&#160; Stop</button>
  <button class="btn btn-clear" id="btn-r" onclick="clearData()" title="Cancel running test and clear all accumulated data">&#x1F5D1;&#160; Clear Data &amp; Reset</button>

  <div style="font-size:11px;color:var(--muted);line-height:1.6;padding-top:4px">
    <b>High concurrency tip:</b> The loadtest container has 65536 file descriptors.
    For 5000+ workers, ensure the proxy also has enough FDs.
  </div>
</div>

<!-- Results panel -->
<div class="results">
  <div class="idle-hint" id="idle">
    <svg width="52" height="52" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1">
      <circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>
    </svg>
    <div style="font-size:15px;font-weight:600">No test running</div>
    <div style="font-size:13px">Pick a preset or fill in the config, then click Start Test.</div>
  </div>

  <div id="live" style="display:none;flex-direction:column;gap:16px">

    <div class="state-bar">
      <div class="dot" id="dot"></div>
      <span id="slabel">—</span>
      <span style="color:var(--muted);font-size:12px;font-weight:400;margin-left:8px" id="etime"></span>
    </div>

    <!-- Active config summary -->
    <div class="info-grid" id="cfg-summary"></div>

    <!-- Big metrics -->
    <div class="cards">
      <div class="card">
        <div class="lbl">Requests / sec</div>
        <div class="val" id="rps">—</div>
      </div>
      <div class="card">
        <div class="lbl">Total Requests</div>
        <div class="val" id="total">—</div>
      </div>
      <div class="card">
        <div class="lbl">Errors</div>
        <div class="val" style="color:var(--red)" id="errs">—</div>
      </div>
    </div>

    <!-- Latency table -->
    <div class="section">
      <div class="section-hdr">Latency Percentiles</div>
      <table>
        <thead><tr><th style="width:60px">pct</th><th style="width:110px">latency</th><th>distribution</th></tr></thead>
        <tbody id="lat-body"></tbody>
      </table>
    </div>

    <!-- Real-time charts -->
    <div id="chart-section" style="display:none;flex-direction:column;gap:12px">
      <div class="section">
        <div class="section-hdr" style="display:flex;align-items:center;justify-content:space-between">
          <span>Requests / Second</span>
          <span id="rps-now" style="font-size:12px;color:var(--green);font-weight:700"></span>
        </div>
        <div class="chart-box" id="chart-rps"></div>
      </div>
      <div class="section">
        <div class="section-hdr" style="display:flex;align-items:center;justify-content:space-between">
          <span>Latency over Time &mdash; p50 / p95 / p99</span>
          <span id="sla-badge" style="font-size:11px;font-weight:700;padding:2px 10px;border-radius:10px"></span>
        </div>
        <div class="chart-box" id="chart-lat"></div>
      </div>
      <div class="section">
        <div class="section-hdr" style="display:flex;align-items:center;justify-content:space-between">
          <span>Error Rate %</span>
          <span id="errpct-now" style="font-size:12px;font-weight:700"></span>
        </div>
        <div style="padding:4px 14px 2px">
          <div class="errpct-bar"><div class="errpct-fill" id="errpct-fill" style="width:0%"></div></div>
        </div>
        <div class="chart-box" id="chart-err"></div>
      </div>
    </div>

    <!-- Status codes -->
    <div class="section">
      <div class="section-hdr">HTTP Status Codes — Proxy Requests</div>
      <div id="codes-body"></div>
    </div>

    <!-- Error source breakdown -->
    <div class="section" id="err-src-section" style="display:none">
      <div class="section-hdr" style="display:flex;align-items:center;justify-content:space-between">
        <span>Error Source Analysis</span>
        <span style="font-size:11px;color:var(--muted);font-weight:400">Where did the errors come from?</span>
      </div>
      <div id="err-src-body"></div>
      <div style="padding:8px 14px 10px;font-size:11px;color:var(--muted);display:grid;grid-template-columns:1fr 1fr;gap:4px 20px" id="err-src-legend">
        <span><b style="color:var(--red)">Network</b> — proxy unreachable (FD exhausted, crash)</span>
        <span><b style="color:var(--yellow)">Gateway 502/503/504</b> — proxy can&#39;t reach upstream</span>
        <span><b style="color:var(--purple)">Upstream 5xx</b> — upstream app error passed through</span>
        <span><b style="color:var(--accent)">Client 4xx</b> — request rejected by proxy (bad Host etc.)</span>
      </div>
    </div>

    <!-- Recent errors -->
    <div class="section" id="errs-section" style="display:none">
      <div class="section-hdr">Recent Errors</div>
      <div class="err-list" id="errs-body"></div>
    </div>

    <!-- Break-point result -->
    <div class="section" id="break-panel" style="display:none;border-color:var(--red)">
      <div class="section-hdr" style="color:var(--red);border-bottom-color:var(--red)">&#x26A1; Break Point Detected</div>
      <div class="break-box" id="break-body"></div>
    </div>

    <!-- collect_logs interference panel (shown only when enabled) -->
    <div id="logs-panel" style="display:none;flex-direction:column;gap:16px">
      <div class="section">
        <div class="section-hdr" style="display:flex;align-items:center;justify-content:space-between">
          <span>collect_logs Interference &mdash; <code style="font-size:10px;font-weight:400">/.svc/collect_logs</code></span>
          <span id="logs-verdict" style="font-size:11px;font-weight:700;padding:2px 8px;border-radius:10px"></span>
        </div>
        <div style="padding:14px">
          <table style="width:100%">
            <thead>
              <tr>
                <th style="width:80px">Metric</th>
                <th style="color:var(--accent)">Proxy requests</th>
                <th style="color:var(--purple)">collect_logs calls</th>
                <th style="width:120px">Delta</th>
              </tr>
            </thead>
            <tbody id="cmp-body"></tbody>
          </table>
        </div>
      </div>
      <div class="section">
        <div class="section-hdr">HTTP Status Codes &mdash; collect_logs calls</div>
        <div id="logs-codes-body"></div>
      </div>
    </div>

  </div>
</div>
</div>

<script>
var timer = null;
var histTimer = null;
var activeMethod = 'GET';
var serverTargetList = [];
var currentConfig = null;

// ── SVG chart renderer ────────────────────────────────────────────────────────
function fmtVal(v, unit) {
  if (unit === 'ms') {
    if (v >= 1000) return (v/1000).toFixed(2) + 's';
    if (v >= 1) return v.toFixed(1) + 'ms';
    return (v*1000).toFixed(0) + 'µs';
  }
  if (unit === 'rps') { return v >= 1000 ? (v/1000).toFixed(1) + 'k' : v.toFixed(0); }
  if (unit === '%') { return v.toFixed(2) + '%'; }
  return v.toFixed(1);
}

function svgChart(elId, pts, series, opts) {
  var el = document.getElementById(elId);
  if (!el) return;
  var W = el.clientWidth || 700;
  var H = opts.h || 150;
  var ml = 52, mr = 12, mt = 14, mb = 28;
  var cw = W - ml - mr, ch = H - mt - mb;

  if (!pts || pts.length < 2) {
    el.innerHTML = '<svg width="100%" height="' + H + '"><text x="' + (W/2) + '" y="' + (H/2+4) + '" text-anchor="middle" fill="#4a5280" font-size="12">Waiting for data…</text></svg>';
    return;
  }

  var ymax = 0.001;
  pts.forEach(function(p) { series.forEach(function(s){ if((p[s.key]||0)>ymax) ymax=p[s.key]; }); });
  if (opts.slaMs > 0) ymax = Math.max(ymax, opts.slaMs);
  ymax *= 1.15;

  var t0 = pts[0].t, t1 = pts[pts.length-1].t, td = Math.max(t1-t0,1);
  function xp(t){ return ml + (t-t0)/td*cw; }
  function yp(v){ return mt + ch*(1 - v/ymax); }

  var defs = '<defs>';
  series.forEach(function(s){
    if(s.fill) defs += '<linearGradient id="g' + elId + s.key + '" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="' + s.color + '" stop-opacity=".3"/><stop offset="100%" stop-color="' + s.color + '" stop-opacity="0"/></linearGradient>';
  });
  defs += '</defs>';

  var grid = '';
  for(var gi=0;gi<=4;gi++){
    var gy=(mt+ch*gi/4).toFixed(1);
    var gv=ymax*(1-gi/4);
    grid += '<line x1="'+ml+'" y1="'+gy+'" x2="'+(ml+cw)+'" y2="'+gy+'" stroke="#1e2438" stroke-width="1"/>';
    grid += '<text x="'+(ml-4)+'" y="'+(parseFloat(gy)+4)+'" fill="#4a5280" font-size="10" text-anchor="end">'+fmtVal(gv,opts.unit)+'</text>';
  }

  var xaxis = '';
  [0,0.5,1].forEach(function(f){
    var t=t0+td*f, x=xp(t).toFixed(1);
    var d=new Date(t);
    var lbl=('0'+d.getMinutes()).slice(-2)+':'+('0'+d.getSeconds()).slice(-2);
    xaxis += '<text x="'+x+'" y="'+(mt+ch+18)+'" fill="#4a5280" font-size="10" text-anchor="middle">'+lbl+'</text>';
  });

  var slaLine = '';
  if(opts.slaMs>0 && opts.slaMs<=ymax){
    var sy=yp(opts.slaMs).toFixed(1);
    slaLine='<line x1="'+ml+'" y1="'+sy+'" x2="'+(ml+cw)+'" y2="'+sy+'" stroke="#ef4444" stroke-width="1.5" stroke-dasharray="6,3" opacity="0.85"/><text x="'+(ml+4)+'" y="'+(parseFloat(sy)-3)+'" fill="#ef4444" font-size="10">SLA</text>';
  }

  var paths = '';
  series.forEach(function(s){
    var d=''; pts.forEach(function(p,i){ var x=xp(p.t).toFixed(1),y=yp(p[s.key]||0).toFixed(1); d+=(i===0?'M':'L')+x+','+y; });
    if(s.fill){ var lx=xp(pts[pts.length-1].t).toFixed(1),fx=xp(pts[0].t).toFixed(1),bot=(mt+ch).toFixed(1); paths+='<path d="'+d+' L'+lx+','+bot+' L'+fx+','+bot+' Z" fill="url(#g'+elId+s.key+')" opacity="0.9"/>'; }
    paths+='<path d="'+d+'" fill="none" stroke="'+s.color+'" stroke-width="'+(s.w||2)+'" stroke-linejoin="round"/>';
  });

  el.innerHTML='<svg width="100%" height="'+H+'" viewBox="0 0 '+W+' '+H+'" preserveAspectRatio="xMidYMid meet">'+defs+grid+xaxis+slaLine+paths+'<rect x="'+ml+'" y="'+mt+'" width="'+cw+'" height="'+ch+'" fill="none" stroke="#2d3448" stroke-width="1"/></svg>';
}

// ── History polling ───────────────────────────────────────────────────────────
function startHistPoll() {
  if(histTimer) clearInterval(histTimer);
  histTimer = setInterval(pollHistory, 1000);
  pollHistory();
}

function stopHistPoll() {
  if(histTimer){ clearInterval(histTimer); histTimer=null; }
}

function pollHistory() {
  fetch('/api/history').then(function(r){return r.json();}).then(renderCharts).catch(function(){});
}

function renderCharts(pts) {
  if(!pts||pts.length<2) return;
  var section = document.getElementById('chart-section');
  section.style.display = 'flex';
  var slaMs = currentConfig ? (currentConfig.sla_ms||100) : 100;

  svgChart('chart-rps', pts, [{key:'rps',color:'#22c55e',fill:true,w:2}], {unit:'rps',h:140});
  svgChart('chart-lat', pts,
    [{key:'p50',color:'#10b981',fill:false,w:2},{key:'p95',color:'#f59e0b',fill:false,w:2},{key:'p99',color:'#ef4444',fill:true,w:2.5}],
    {unit:'ms', h:160, slaMs:slaMs});
  svgChart('chart-err', pts, [{key:'ep',color:'#ef4444',fill:true,w:2}], {unit:'%',h:100});

  var last = pts[pts.length-1];
  document.getElementById('rps-now').textContent = fmtVal(last.rps,'rps') + ' req/s';

  var errPctEl = document.getElementById('errpct-now');
  var fillEl = document.getElementById('errpct-fill');
  errPctEl.textContent = last.ep.toFixed(2) + '%';
  errPctEl.style.color = last.ep > 5 ? 'var(--red)' : last.ep > 1 ? 'var(--yellow)' : 'var(--green)';
  fillEl.style.width = Math.min(100, last.ep*5) + '%';

  var slaOk = last.p99 <= slaMs;
  var badge = document.getElementById('sla-badge');
  if(slaMs>0){
    badge.textContent = slaOk ? '✓ SLA OK (<'+slaMs+'ms)' : '✗ SLA BREACH ('+last.p99.toFixed(0)+'ms)';
    badge.style.background = slaOk ? '#14532d' : '#450a0a';
    badge.style.color = slaOk ? 'var(--green)' : 'var(--red)';
  }
}

// ── Target cards ──────────────────────────────────────────────────────────────
var TARGET_ICONS = {'rust-proxy-server': '🦀', 'nginx': '⚡'};

function loadTargets() {
  fetch('/api/targets').then(function(r){return r.json();}).then(function(list) {
    if (!list || list.length === 0) return;
    serverTargetList = list;
    var grid = document.getElementById('target-grid');
    var html = '';
    for (var i = 0; i < list.length; i++) {
      var t = list[i];
      var icon = TARGET_ICONS[t.name] || '🖥';
      html += '<div class="target-card" id="tc-' + i + '" onclick="selectTarget(' + i + ')">'
        + '<div class="tc-icon">' + icon + '</div>'
        + '<span class="tc-name">' + t.name + '</span>'
        + '<span class="tc-url">' + t.url + '</span>'
        + '</div>';
    }
    grid.innerHTML = html;
    document.getElementById('target-section').style.display = 'block';
    // Auto-select first target
    selectTarget(0);
  }).catch(function(){});
}

function selectTarget(idx) {
  var t = serverTargetList[idx];
  if (!t) return;
  document.getElementById('url').value  = t.url;
  document.getElementById('host').value = t.host || '';
  document.querySelectorAll('.target-card').forEach(function(c,i){
    c.classList.toggle('active', i === idx);
  });
}

var PRESETS = {
  warmup:   {concurrency:10,   duration_s:30,   max_rps:0, ramp_up_s:0,  timeout_s:30, method:'GET', no_keepalive:false, sla_ms:100},
  baseline: {concurrency:100,  duration_s:60,   max_rps:0, ramp_up_s:5,  timeout_s:30, method:'GET', no_keepalive:false, sla_ms:100},
  load:     {concurrency:500,  duration_s:60,   max_rps:0, ramp_up_s:10, timeout_s:30, method:'GET', no_keepalive:false, sla_ms:100},
  stress:   {concurrency:2000, duration_s:30,   max_rps:0, ramp_up_s:15, timeout_s:10, method:'GET', no_keepalive:false, sla_ms:200},
  spike:    {concurrency:5000, duration_s:10,   max_rps:0, ramp_up_s:0,  timeout_s:5,  method:'GET', no_keepalive:false, sla_ms:500},
  ramp:     {concurrency:50,   duration_s:3600, max_rps:0, ramp_up_s:0,  timeout_s:10, method:'GET', no_keepalive:false, sla_ms:200,
             ramp_mode:true, ramp_start_conc:10, ramp_end_conc:2000, ramp_step_size:50, ramp_step_s:10, ramp_stop_err_pct:10},
};

function applyPreset(name) {
  var p = PRESETS[name];
  document.getElementById('conc').value = p.concurrency;
  document.getElementById('dur').value = p.duration_s;
  document.getElementById('maxrps').value = p.max_rps;
  document.getElementById('rampup').value = p.ramp_up_s;
  document.getElementById('timeout').value = p.timeout_s;
  document.getElementById('nkv').checked = p.no_keepalive;
  document.getElementById('sla-ms').value = p.sla_ms || 100;
  setMethod(p.method);
  // ramp mode
  var rampMode = !!p.ramp_mode;
  document.getElementById('ramp-mode').checked = rampMode;
  var rampOpts = document.getElementById('ramp-opts');
  rampOpts.style.display = rampMode ? 'flex' : 'none';
  if (rampMode) {
    document.getElementById('ramp-start').value  = p.ramp_start_conc  || 10;
    document.getElementById('ramp-end').value    = p.ramp_end_conc    || 2000;
    document.getElementById('ramp-step').value   = p.ramp_step_size   || 50;
    document.getElementById('ramp-step-s').value = p.ramp_step_s      || 10;
    document.getElementById('ramp-err-pct').value= p.ramp_stop_err_pct|| 10;
  }
  document.querySelectorAll('.preset-btn').forEach(function(b){b.classList.remove('active');});
  document.querySelectorAll('.preset-btn').forEach(function(b,i){
    var n=['warmup','baseline','load','stress','spike','ramp'];
    if (n[i]===name) b.classList.add('active');
  });
  if (p.ramp_up_s > 0 || p.timeout_s !== 30 || rampMode) {
    document.getElementById('adv-body').style.display = 'flex';
    document.getElementById('adv-body').style.flexDirection = 'column';
    document.getElementById('adv-body').style.gap = '12px';
    document.getElementById('adv-arrow').textContent = '▼';
  }
}

function setConc(n) { document.getElementById('conc').value = n; }

function setMethod(m) {
  activeMethod = m;
  ['GET','POST','PUT','HEAD'].forEach(function(x){
    document.getElementById('m-'+x).classList.toggle('active', x === m);
  });
}

function toggleRampMode() {
  var on = document.getElementById('ramp-mode').checked;
  var opts = document.getElementById('ramp-opts');
  opts.style.display = on ? 'flex' : 'none';
  opts.style.flexDirection = 'column';
  opts.style.gap = '6px';
}

function toggleLoggingImpact() {
  var on = document.getElementById('logging-impact').checked;
  if (on) {
    document.getElementById('cl-enabled').checked = true;
    toggleClInterval();
  }
}

function toggleClInterval() {
  var row = document.getElementById('cl-interval-row');
  row.style.display = document.getElementById('cl-enabled').checked ? 'block' : 'none';
}

function toggleAdv() {
  var body = document.getElementById('adv-body');
  var arrow = document.getElementById('adv-arrow');
  if (body.style.display === 'none') {
    body.style.display = 'flex';
    body.style.flexDirection = 'column';
    body.style.gap = '12px';
    arrow.textContent = '▼';
  } else {
    body.style.display = 'none';
    arrow.textContent = '▶';
  }
}

function buildConfig() {
  var clEnabled = document.getElementById('cl-enabled').checked;
  var clMs = clEnabled ? (+document.getElementById('cl-interval').value || 200) : 0;
  var rampMode = document.getElementById('ramp-mode').checked;
  var loggingImpact = document.getElementById('logging-impact').checked;
  if (loggingImpact && !clEnabled) { clMs = 200; }
  return {
    url:               document.getElementById('url').value,
    host:              document.getElementById('host').value,
    concurrency:       +document.getElementById('conc').value || 50,
    duration_s:        +document.getElementById('dur').value || 30,
    max_rps:           +document.getElementById('maxrps').value || 0,
    ramp_up_s:         +document.getElementById('rampup').value || 0,
    timeout_s:         +document.getElementById('timeout').value || 30,
    method:            activeMethod,
    no_keepalive:      document.getElementById('nkv').checked,
    extra_headers:     document.getElementById('headers').value,
    collect_logs_ms:   clMs,
    sla_ms:            +document.getElementById('sla-ms').value || 100,
    mixed_methods:     document.getElementById('mixed-methods').checked,
    logging_impact:    loggingImpact,
    ramp_mode:         rampMode,
    ramp_start_conc:   rampMode ? (+document.getElementById('ramp-start').value  || 10)   : 0,
    ramp_end_conc:     rampMode ? (+document.getElementById('ramp-end').value    || 2000) : 0,
    ramp_step_size:    rampMode ? (+document.getElementById('ramp-step').value   || 50)   : 0,
    ramp_step_s:       rampMode ? (+document.getElementById('ramp-step-s').value || 10)   : 0,
    ramp_stop_err_pct: rampMode ? (+document.getElementById('ramp-err-pct').value|| 10)   : 0,
  };
}

function startTest() {
  var body = buildConfig();
  fetch('/api/run', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)})
    .then(function(r){return r.json();})
    .then(function(d){if(d.error)alert(d.error);else beginPolling();})
    .catch(function(e){alert('Error: '+e);});
}

function stopTest() { fetch('/api/stop', {method:'POST'}); }

function clearData() {
  fetch('/api/reset', {method:'POST'})
    .then(function(r){return r.json();})
    .then(function(d){
      if(d.error){alert(d.error);return;}
      if(timer){clearInterval(timer);timer=null;}
      stopHistPoll();
      document.getElementById('idle').style.display='flex';
      document.getElementById('live').style.display='none';
      document.getElementById('btn-s').style.display='block';
      document.getElementById('btn-x').style.display='none';
      document.getElementById('err-src-section').style.display='none';
      document.getElementById('errs-section').style.display='none';
      document.getElementById('break-panel').style.display='none';
      document.getElementById('chart-section').style.display='none';
      document.getElementById('logs-panel').style.display='none';
    })
    .catch(function(e){alert('Error: '+e);});
}

function beginPolling() {
  document.getElementById('idle').style.display = 'none';
  document.getElementById('live').style.display = 'flex';
  document.getElementById('btn-s').style.display = 'none';
  document.getElementById('btn-x').style.display = 'block';
  if (timer) clearInterval(timer);
  timer = setInterval(poll, 1000);
  poll();
  startHistPoll();
}

function poll() {
  fetch('/api/status').then(function(r){return r.json();}).then(render).catch(function(){});
}

function fmtMs(ms) {
  if (ms === null || ms === undefined) return '—';
  if (ms < 1) return (ms*1000).toFixed(0) + ' µs';
  if (ms < 1000) return ms.toFixed(2) + ' ms';
  return (ms/1000).toFixed(2) + ' s';
}

function render(d) {
  var dot = document.getElementById('dot');
  dot.className = 'dot' + (d.state==='running' ? ' running' : d.state==='done' ? ' done' : '');
  document.getElementById('slabel').textContent = d.state.toUpperCase();
  document.getElementById('etime').textContent = d.elapsed_s > 0 ? d.elapsed_s.toFixed(1) + 's elapsed' : '';
  document.getElementById('rps').textContent = d.rps ? Math.round(d.rps).toLocaleString() : '—';
  document.getElementById('total').textContent = d.total ? d.total.toLocaleString() : '—';
  document.getElementById('errs').textContent = (d.errors !== undefined) ? d.errors.toLocaleString() : '—';

  // Config summary
  if (d.config) {
    var c = d.config;
    var rpsLabel = c.max_rps > 0 ? c.max_rps.toLocaleString()+' cap' : 'unlimited';
    var rampLabel = c.ramp_up_s > 0 ? c.ramp_up_s+'s ramp' : 'instant';
    var workerVal;
    if (c.ramp_mode) {
      var live = d.current_conc || 0;
      workerVal = live.toLocaleString() + ' / ' + (c.ramp_end_conc||2000).toLocaleString() + ' (ramping)';
    } else {
      workerVal = c.concurrency ? c.concurrency.toLocaleString() : '—';
    }
    document.getElementById('cfg-summary').innerHTML =
      mkInfo('Workers', workerVal) +
      mkInfo('Duration', (c.duration_s||'?') + 's') +
      mkInfo('Method', c.mixed_methods ? 'Mixed (GET/POST/HEAD)' : (c.method || 'GET')) +
      mkInfo('Max RPS', rpsLabel) +
      mkInfo('Ramp-up', rampLabel) +
      mkInfo('Timeout', (c.timeout_s||30) + 's');
  }

  // Latency table
  var base = Math.max(d.p99_ms || 0, 0.01);
  var rows = [
    ['p50', d.p50_ms, 'var(--green)'],
    ['p95', d.p95_ms, 'var(--yellow)'],
    ['p99', d.p99_ms, 'var(--red)'],
    ['min', d.min_ms, 'var(--muted)'],
    ['avg', d.avg_ms, 'var(--accent)'],
    ['max', d.max_ms, 'var(--purple)'],
  ];
  document.getElementById('lat-body').innerHTML = rows.map(function(r) {
    var pct = Math.min(100, ((r[1]||0)/base)*100);
    return '<tr>'
      + '<td style="color:var(--muted);font-size:11px;font-weight:700;width:60px">' + r[0] + '</td>'
      + '<td style="font-weight:700;width:110px">' + fmtMs(r[1]) + '</td>'
      + '<td><div class="bar-wrap"><div class="bar-fill" style="width:' + pct + '%;background:' + r[2] + '"></div></div></td>'
      + '</tr>';
  }).join('');

  // Status codes
  var codes = d.codes || {};
  var totalReqs = 0;
  for (var k in codes) totalReqs += codes[k];
  if (d.errors) totalReqs += d.errors;
  if (totalReqs === 0) totalReqs = 1;
  var entries = [];
  for (var k in codes) entries.push([k, codes[k]]);
  entries.sort(function(a,b){return a[0].localeCompare(b[0]);});
  if (d.errors) entries.push(['error', d.errors]);
  var html = entries.length ? entries.map(function(e) {
    var code = e[0], cnt = e[1];
    var pct = Math.min(100, (cnt/totalReqs)*100);
    var cls = code==='error' ? 'cerr' : code[0]==='2' ? 'c2' : code[0]==='4' ? 'c4' : 'c5';
    return '<div class="code-row">'
      + '<span class="code-lbl">' + code + '</span>'
      + '<div style="flex:1;background:var(--border);height:7px;border-radius:4px;overflow:hidden"><div class="code-bar ' + cls + '" style="width:' + pct + '%;height:100%"></div></div>'
      + '<span class="code-cnt">' + cnt.toLocaleString() + ' (' + pct.toFixed(1) + '%)</span>'
      + '</div>';
  }).join('') : '<div style="padding:14px;color:var(--muted)">No requests recorded yet…</div>';
  document.getElementById('codes-body').innerHTML = html;

  // ── collect_logs interference panel ─────────────────────────────────────
  var logsPanel = document.getElementById('logs-panel');
  if (d.logs_stats) {
    logsPanel.style.display = 'flex';
    var ls = d.logs_stats;
    var rows = [
      ['p50', d.p50_ms, ls.p50_ms],
      ['p95', d.p95_ms, ls.p95_ms],
      ['p99', d.p99_ms, ls.p99_ms],
      ['avg', d.avg_ms, ls.avg_ms],
    ];
    document.getElementById('cmp-body').innerHTML = rows.map(function(r) {
      var proxy = r[1] || 0, logs = r[2] || 0;
      var delta = logs - proxy;
      var sign  = delta > 0 ? '+' : '';
      var color = Math.abs(delta) < proxy * 0.1 ? 'var(--green)' : delta < 0 ? 'var(--green)' : 'var(--yellow)';
      return '<tr>'
        + '<td style="color:var(--muted);font-size:11px;font-weight:700;width:80px">' + r[0] + '</td>'
        + '<td style="font-weight:700;color:var(--accent)">' + fmtMs(proxy) + '</td>'
        + '<td style="font-weight:700;color:var(--purple)">' + fmtMs(logs) + '</td>'
        + '<td style="font-size:12px;color:' + color + '">' + sign + fmtMs(delta) + '</td>'
        + '</tr>';
    }).join('') + '<tr>'
      + '<td style="color:var(--muted);font-size:11px;font-weight:700">total</td>'
      + '<td style="color:var(--accent)">' + (d.total||0).toLocaleString() + '</td>'
      + '<td style="color:var(--purple)">' + (ls.total||0).toLocaleString() + '</td>'
      + '<td></td>'
      + '</tr>';

    // Verdict badge
    var verdict = document.getElementById('logs-verdict');
    var p95delta = (ls.p95_ms||0) - (d.p95_ms||1);
    var pct = Math.abs(p95delta) / Math.max(d.p95_ms||1, 0.01) * 100;
    if (pct < 10) {
      verdict.textContent = '✓ No interference';
      verdict.style.background = '#14532d'; verdict.style.color = 'var(--green)';
    } else if (pct < 30) {
      verdict.textContent = '~ Slight impact';
      verdict.style.background = '#451a03'; verdict.style.color = 'var(--yellow)';
    } else {
      verdict.textContent = '✗ Impact detected';
      verdict.style.background = '#450a0a'; verdict.style.color = 'var(--red)';
    }

    // collect_logs status codes
    var lcodes = ls.codes || {};
    var ltotal = ls.total || 1;
    var lentries = [];
    for (var k in lcodes) lentries.push([k, lcodes[k]]);
    lentries.sort(function(a,b){return a[0].localeCompare(b[0]);});
    if (ls.errors) lentries.push(['error', ls.errors]);
    var lhtml = lentries.length ? lentries.map(function(e) {
      var code = e[0], cnt = e[1];
      var pct2 = Math.min(100, (cnt/ltotal)*100);
      var cls = code==='error' ? 'cerr' : code[0]==='2' ? 'c2' : code[0]==='4' ? 'c4' : 'c5';
      return '<div class="code-row">'
        + '<span class="code-lbl">' + code + '</span>'
        + '<div style="flex:1;background:var(--border);height:7px;border-radius:4px;overflow:hidden"><div class="code-bar ' + cls + '" style="width:' + pct2 + '%;height:100%"></div></div>'
        + '<span class="code-cnt">' + cnt.toLocaleString() + ' (' + pct2.toFixed(1) + '%)</span>'
        + '</div>';
    }).join('') : '<div style="padding:14px;color:var(--muted)">No calls yet…</div>';
    document.getElementById('logs-codes-body').innerHTML = lhtml;
  } else {
    logsPanel.style.display = 'none';
  }

  // Store config for chart SLA reference
  if (d.config) currentConfig = d.config;

  // Error source breakdown
  var errSrcSection = document.getElementById('err-src-section');
  var errSrcBody = document.getElementById('err-src-body');
  var codes3 = d.codes || {};
  var gwErrs = (codes3['502']||0)+(codes3['503']||0)+(codes3['504']||0);
  var upErrs = 0, clErrs = 0;
  for(var ck in codes3){
    var cn=parseInt(ck,10);
    if(cn>=500 && cn!==502 && cn!==503 && cn!==504) upErrs+=codes3[ck];
    if(cn>=400 && cn<500) clErrs+=codes3[ck];
  }
  var netErrs = d.errors || 0;
  var totalAll = d.total || 1;
  if(errSrcSection && (netErrs+gwErrs+upErrs+clErrs) > 0){
    errSrcSection.style.display='block';
    var srcRows=[
      ['Network (TCP)','Proxy unreachable / FD exhausted',netErrs,'var(--red)'],
      ['Gateway (502/503/504)','Proxy → upstream failed',gwErrs,'var(--yellow)'],
      ['Upstream (5xx)','App error passed through proxy',upErrs,'var(--purple)'],
      ['Client (4xx)','Request rejected by proxy',clErrs,'var(--accent)'],
    ];
    errSrcBody.innerHTML = srcRows.filter(function(row){return row[2]>0;}).map(function(row){
      var pct3=(row[2]/totalAll*100);
      return '<div class="code-row">'
        +'<div style="min-width:190px"><span style="font-size:12px;font-weight:700;color:'+row[3]+'">'+row[0]+'</span>'
        +'<div style="font-size:10px;color:var(--muted);margin-top:1px">'+row[1]+'</div></div>'
        +'<div style="flex:1;background:var(--border);height:7px;border-radius:4px;overflow:hidden;margin:0 10px">'
        +'<div style="width:'+Math.min(100,pct3*5)+'%;height:100%;border-radius:4px;background:'+row[3]+'"></div></div>'
        +'<span class="code-cnt">'+row[2].toLocaleString()+' ('+pct3.toFixed(2)+'%)</span>'
        +'</div>';
    }).join('');
  } else if(errSrcSection){
    errSrcSection.style.display='none';
  }

  // Recent errors
  var errSection = document.getElementById('errs-section');
  if (d.recent_errors && d.recent_errors.length > 0) {
    errSection.style.display = 'block';
    document.getElementById('errs-body').innerHTML = d.recent_errors.slice(-10).reverse().map(function(e){
      return '<div class="err-item">' + e + '</div>';
    }).join('');
  } else {
    errSection.style.display = 'none';
  }

  // Break / ramp-stop panel
  var bp = d.break_point;
  var bpPanel = document.getElementById('break-panel');
  if (bp) {
    var isRamp = d.config && d.config.ramp_mode;
    bpPanel.style.display = 'block';
    document.getElementById('break-body').innerHTML =
      '<div class="break-stat"><span class="bk">RPS at stop</span><span class="bv" style="color:var(--red)">' + Math.round(bp.rps).toLocaleString() + '</span></div>'
      + (isRamp ? '<div class="break-stat"><span class="bk">Workers at stop</span><span class="bv" style="color:var(--red)">' + (d.current_conc||'—').toLocaleString() + '</span></div>' : '')
      + '<div class="break-stat"><span class="bk">Error rate</span><span class="bv" style="color:var(--red)">' + bp.err_pct.toFixed(1) + '%</span></div>'
      + '<div class="break-stat"><span class="bk">Elapsed</span><span class="bv" style="color:var(--muted)">' + bp.at_s.toFixed(0) + 's</span></div>'
      + '<div style="font-size:12px;color:var(--muted);margin-top:4px">'
      + (isRamp ? 'Ramp stopped: error threshold reached at the above concurrency level.'
                : 'The proxy exceeded the error threshold at the above load level.')
      + '</div>';
  } else {
    bpPanel.style.display = 'none';
  }

  if (d.state==='done') {
    clearInterval(timer); timer=null;
    stopHistPoll();
    document.getElementById('btn-s').style.display='block';
    document.getElementById('btn-x').style.display='none';
  }
  if (d.state==='idle') {
    clearInterval(timer); timer=null;
    stopHistPoll();
    document.getElementById('idle').style.display='flex';
    document.getElementById('live').style.display='none';
    document.getElementById('btn-s').style.display='block';
    document.getElementById('btn-x').style.display='none';
  }
}

function mkInfo(k, v) {
  return '<div class="info-card"><div class="k">' + k + '</div><div class="v">' + v + '</div></div>';
}

// Resume if test is already running (page refresh)
fetch('/api/status').then(function(r){return r.json();}).then(function(d){
  if (d.state==='running' || d.state==='done') {
    document.getElementById('idle').style.display='none';
    document.getElementById('live').style.display='flex';
    if (d.config) {
      if (d.config.url) document.getElementById('url').value = d.config.url;
      if (d.config.host) document.getElementById('host').value = d.config.host;
      if (d.config.concurrency) document.getElementById('conc').value = d.config.concurrency;
      if (d.config.method) setMethod(d.config.method);
      if (d.config.max_rps !== undefined) document.getElementById('maxrps').value = d.config.max_rps;
      if (d.config.ramp_up_s !== undefined) document.getElementById('rampup').value = d.config.ramp_up_s;
      if (d.config.timeout_s !== undefined) document.getElementById('timeout').value = d.config.timeout_s;
    }
    if (d.state==='running') beginPolling(); else render(d);
  }
}).catch(function(){});

// ── Boot ──────────────────────────────────────────────────────────────────────
loadTargets();
</script>
</body>
</html>`
