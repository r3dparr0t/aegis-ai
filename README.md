# Aegis-AI 🛡️

A self-correcting **LLM agent for web security fuzzing**, written in Rust.
Point it at a lab, and it plans an attack, executes it against a real target,
evaluates the response, and refines its strategy — attempt after attempt —
until it succeeds or runs out of budget.

The agent runs against any LLM (local Ollama by default), stores what it
learns in SQLite, and ships with a browser-based control panel.

---

## What it does

```
┌────────────────────────────────────────────────────────────┐
│                       Attempt N                            │
│                                                            │
│   ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌───────┐   │
│   │ Retrieve │──▶│ Generate │──▶│ Execute  │──▶│Evaluate│  │
│   │ lessons  │   │ payload  │   │ against  │   │real   │   │
│   │(SQLite)  │   │ via LLM  │   │ target   │   │reply  │   │
│   └──────────┘   └──────────┘   └──────────┘   └───┬───┘   │
│                                                    │       │
│                                          pass ◀────┴──▶ fail│
│                                                     │       │
│                                              ┌──────▼─────┐ │
│                                              │ Distill    │ │
│                                              │ lesson,    │ │
│                                              │ store,     │ │
│                                              │ retry      │ │
│                                              └────────────┘ │
└────────────────────────────────────────────────────────────┘
```

1. **Retrieve** — pull relevant past lessons for this task from SQLite.
2. **Generate** — ask the LLM for an attack payload (JSON with `endpoint` and `body`).
3. **Execute** — POST it to the real vulnerable target over HTTP.
4. **Evaluate** — check the actual target response (flag? latency? status code?).
5. **Reflect** — on failure, distill a short, actionable lesson and store it.
6. Repeat until the target responds with the success marker or `max_attempts` is reached.

The LLM never sees its own output as ground truth — only real HTTP responses
from the real target count.

---

## Features

- **Adaptive reflexion loop** — learns from real failures in seconds, no fine-tuning.
- **Episodic memory (SQLite)** — every execution, attempt, and lesson is stored and queryable.
- **Task-scoped memory** — lessons for `ssrf_ip_encoding_bypass` don't leak into `ssrf_webhook_blacklist`.
- **YAML-driven labs** — adding a new lab is one file, zero Rust changes.
- **Multi-provider LLM transport** — Ollama, OpenAI-compatible (OpenAI, DeepSeek, Groq, …), TypeSafe.
- **Auto-discovery of API keys** — reads `.env` × `providers.toml` and lists what's available.
- **Deterministic transforms via LLM intent** — the model chooses *what* (`"ip_encoding": "hex"`), Rust does the math.
- **Event-driven architecture** — one `EngineObserver` trait; CLI and Web both consume the same stream.
- **Web control panel** — sidebar of labs with live state, YAML editor, JSON report viewer, live model switcher.
- **Optional CLI** — `cargo run -- --cli lab1_fetch` for scripted runs.

---

## Quick Start

### 1. Prerequisites

- **Rust** (stable 1.75+)
- **Ollama** (or an API key for OpenAI-compatible providers)
- **Docker** + Docker Compose (for the example labs)

### 2. Start the LLM

```bash
ollama pull qwen2.5:3b
ollama serve
```

A 3B model is enough for the built-in labs.

### 3. Start the vulnerable lab network

```bash
docker compose up -d --build
```

This brings up:
- `vulnerable-api` on port `5000` (the SSRF target)
- `internal-admin` on the lab network (the secret to reach)

### 4. Run Aegis-AI

```bash
cargo run
```

The web panel opens at **http://127.0.0.1:7777**. Pick a lab, hit **Run**, watch
the attempt stream live. Every run produces a report under `reports/`.

---

## The Built-in Labs

| # | Endpoint | What it teaches |
|---|---|---|
| 1 | `POST /api/v1/fetch` | Baseline SSRF — no filter at all |
| 2 | `POST /api/v2/webhook` | Naive blacklist on `localhost` / `127.0.0.1` |
| 3 | *(memory isolation test)* | Same as Lab 1, but a fresh `task_type` — proves lessons don't leak |
| 4 | `POST /api/v3/strict` | Blocks hostname **and** dotted-quad IPv4; only an encoded IP bypasses |
| 5 | `POST /api/v4/blind-time` | Blind SSRF — response body is discarded, only latency matters |

Lab 4 demonstrates the **declarative transform** pattern: the model outputs
the plain IP plus `"ip_encoding": "decimal"`, and Rust performs the numeric
conversion before sending. The LLM picks the strategy; the code does the
arithmetic.

## Real CVEs (via vulhub)

Beyond the training labs, Aegis can exploit **real CVEs** in isolated
Docker containers from [vulhub](https://github.com/vulhub/vulhub).

| CVE | Target | Technique | Status |
|---|---|---|---|
| CVE-2021-41773 | Apache 2.4.49 | Path traversal via `.%2e` → RCE through mod_cgi | ✅ passing |

Each CVE is a YAML lab with `target.kind: raw`. Unlike the SSRF labs,
these use a **raw TCP executor** that sends the HTTP request byte-for-byte —
bypassing the WHATWG URL normalization in the `url` crate, which would
otherwise collapse `.%2e` to `..` before the request leaves the client.
(The same reason `curl` has `--path-as-is`.)

Example — the Apache CVE lab:

```yaml
target:
  kind: raw
  url: "http://localhost:8080"
  method: POST
  headers:
    Content-Type: text/plain

evaluator:
  kind: regex
  pattern: "(root:.*:0:0:|uid=\\d+\\([a-z0-9_]+\\))"
```
---

## Adding a Lab

Drop a YAML file into `labs/`:

```yaml
meta:
  id: lab6_my_scenario
  name: "Lab 6 — My scenario"
  description: "One-line summary shown in the UI."
  order: 6

max_attempts: 5

target:
  url: http://localhost:5000/api/v5/whatever
  body_key: target_url

internal_target:
  host: internal-admin
  port: 8080
  path: /admin/secret-flag

task:
  task_type: ssrf_v5_whatever
  default_goal: "Reach {{target_url}} and retrieve the flag."

evaluator:
  kind: flag
  marker: "FLAG{"

system_prompt: |
  You are an automated SSRF exploitation agent testing a lab API.
  ...
```

Restart the app, and the new lab appears in the sidebar. No code changes.

### Placeholders in prompts and goals

| Placeholder | Expands to |
|---|---|
| `{{target_url}}` | `http://host:port/path` |
| `{{target_host}}` | `host` |
| `{{target_port}}` | `port` |
| `{{target_path}}` | `path` |

### Evaluators

```yaml
evaluator:
  kind: flag
  marker: "FLAG{"        # success = response body contains this
```

```yaml
evaluator:
  kind: time_delay
  threshold_ms: 2500     # success = response latency ≥ threshold
```

---

## CLI Mode

The web panel is the default. For scripted runs:

```bash
# Interactive menu
cargo run -- --cli

# Run a single lab by id, exit
cargo run -- --cli lab1_fetch
```

---

## Architecture

```
src/
├── domain/           Traits: LlmProvider, Evaluator, TargetExecutor, types
├── engine/
│   ├── orchestrator.rs   The Generate→Execute→Evaluate→Reflect loop
│   └── state.rs          ExecutionState (Preparing, Generating, Executing, …)
├── executor/
│   ├── http.rs           HTTP transport
│   ├── json.rs           LLM output parsing (handles </think>, fences, noise)
│   ├── payload.rs        Interprets `ip_encoding` and rewrites the payload
│   └── ip.rs             IPv4 → decimal / hex / octal (pure functions)
├── evaluator/        FlagEvaluator, TimeDelayEvaluator, RegexEvaluator
├── provider/         OllamaProvider, OpenAiCompatibleProvider, TypesafeProvider
├── memory/           SQLite repo: executions, attempts, lessons, lesson_usage
├── labs/
│   ├── spec.rs       YAML schema (LabSpec)
│   ├── lab.rs        Lab = spec + state; encapsulates everything lab-specific
│   ├── registry.rs   Load all YAMLs from labs/
│   └── runner.rs     Orchestration; owns the global run lock
├── events.rs         Event enum + EngineObserver trait + ConsoleObserver
├── selection.rs      Provider/API-key discovery and menu
└── web/
    ├── state.rs      WebState (live log) + observers
    └── handlers.rs   REST + WebSocket handlers

labs/                YAML lab definitions (edited at runtime from the web panel)
static/index.html    Single-file web UI
reports/             Generated JSON + Markdown reports
migrations/          SQLite schema
```

### Key abstractions

- **`LlmProvider`** — `generate(&LlmRequest) -> LlmResponse`. Swap LLMs by changing one implementation.
- **`TargetExecutor`** — `execute(&payload) -> ExecutionOutcome`. The only thing that touches the real target.
- **`Evaluator`** — `evaluate(EvaluationInput) -> EvaluationResult`. Decides pass/fail.
- **`EngineObserver`** — `on_event(Event)`. The CLI, Web panel, and any future consumer subscribe to the same stream.
- **`Lab`** — one immutable `spec` (from YAML) + one `Mutex<LabState>`. The single source of truth for lab state.

---

## Configuration

### API keys — `.env`

```env
OPENAI_API_KEY=sk-...
TYPESAFE_API_KEY=tsk_...
```

Never commit `.env`. `.env.example` lists the expected names.

### Providers — `providers.toml`

```toml
[[provider]]
env_var       = "OPENAI_API_KEY"
name          = "OpenAI"
base_url      = "https://api.openai.com/v1"
default_model = "gpt-4o-mini"
kind          = "openai_compatible"

[[provider]]
env_var       = "TYPESAFE_API_KEY"
name          = "TypeSafe"
base_url      = "https://api.typesafe.ai/v1"
default_model = "jev-latest"
kind          = "typesafe"
```

Add a provider here without recompiling.

---

## Roadmap

- [x] YAML-driven labs
- [x] Event-driven observer (CLI + Web from one source)
- [x] Per-lab state + global run lock
- [x] Live YAML editing from the web panel
- [x] Model switcher in the web panel
- [ ] **Vulhub integration** — turn real CVEs into YAML labs
- [ ] Generalized HTTP executor (headers, raw body, query params, method)
- [ ] Out-of-band evaluator (DNS/HTTP callback)
- [ ] Multi-lab queue (run-all with real sequencing)
- [ ] Anthropic + Gemini providers

---

## License

MIT
