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
- **Optional per-lab shell execution** — a lab can expose a single fixed script to the
  model via `{{shell_output [...]}}` placeholders. The script path is set in YAML and
  never taken from model output; arguments are passed as argv, not via a shell.
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

For multi-step CVEs whose payload contains large base64 blobs (e.g.
CVE-2021-3129 — Laravel Ignition RCE), a lab can optionally enable the
**shell block** (see below) so the model can call a fixed helper script to
generate the blob dynamically, instead of hard-coding it in the YAML.

---

## Shell Execution (optional)

Some exploits — log-poisoning chains, PHAR deserialization, anything that
needs a large dynamically-generated blob — are brittle when the entire
payload has to be baked into the YAML. The optional `shell` block lets a
lab expose **one fixed script** to the model. The model can then ask Aegis
to run that script with its own arguments, and Aegis substitutes the real
script output back into the model's response.

### YAML

```yaml
shell:
  script_path: "/abs/path/to/helper.py"   # must exist and be executable
  timeout_secs: 15
```

If the `shell` block is absent, the lab behaves exactly as before — the
model's prompt is unchanged, no shell-related instruction is injected, and
no script is ever spawned.

### Placeholder contract

When `shell` is configured, Aegis appends an instruction block to the
system prompt telling the model to emit exactly **one** placeholder of the
form:

```
{{shell_output ["arg1", "arg2", ...]}}
```

**Inside a JSON string value of the response**, e.g.:

```json
{"method":"POST","path":"/exec","body":"{{shell_output [\"id\"]}}"}
```

### What Aegis guarantees

- **Script path is fixed.** It comes from the YAML, is validated at lab
  load time (exists + executable), and is never taken from model output.
- **No shell interpretation of arguments.** The model's strings are passed
  to the script as `argv` — `Command::new(script).args(&args)` — not through
  `sh -c`. No metacharacters, no globbing, no substitution.
- **One call per attempt.** If the model emits more than one placeholder in
  a single response, **none** are executed and the attempt is consumed
  (a `shell_failed` lesson is recorded for the next attempt).
- **JSON-safe substitution.** The script's stdout is passed through
  `serde_json::to_string` before being spliced into the response, so quotes,
  backslashes, newlines, tabs, and control bytes all survive as valid JSON.
- **Real timeout.** The script runs in its own process group
  (`process_group(0)` on Unix). On timeout, `SIGKILL` is sent to the whole
  group, so grandchildren (e.g. a `sleep` inside a wrapper) are killed too.
- **No new attempt, no separate evaluator.** The substituted response flows
  through the existing pipeline (`extract_json_object` → `apply_transforms`
  → `TargetExecutor` → `Evaluator`). Shell calls are never sent to the
  evaluator and never count as their own attempt.

A worked example: a Laravel Ignition CVE lab can hard-code the four
execute-solution requests, but put `{{shell_output ["id"]}}` in the step
whose body carries the base64 PHAR. A tiny helper script
(`phpggc -f -b Monolog/RCE1 system id`) generates the blob; Aegis splices
it in; the same 4-step chain runs; the regex evaluator looks at the final
HTTP response.

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

# Optional — only for labs that need a fixed helper script.
# shell:
#   script_path: "/abs/path/to/helper.py"
#   timeout_secs: 15

system_prompt: |
  You are an automated SSRF exploitation agent testing a lab API.
  ...
```

Restart the app, and the new lab appears in the sidebar. No code changes.

### Placeholders in prompts and goals

| Placeholder | Expands to | Where it's resolved |
|---|---|---|
| `{{target_url}}` | `http://host:port/path` | When rendering `system_prompt` / `default_goal` |
| `{{target_host}}` | `host` | same |
| `{{target_port}}` | `port` | same |
| `{{target_path}}` | `path` | same |
| `{{shell_output [...]}}` | Real script stdout (JSON-escaped) | **In the model's response, on every attempt**, when `shell` is configured |

The first four are filled in **before** the prompt is sent. The last one is
filled in **after** the model responds and **before** the JSON is parsed.

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

```yaml
evaluator:
  kind: regex
  pattern: 'uid=\d+\([a-zA-Z0-9_-]+\)'   # success = regex matches the response body
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
│   ├── raw.rs            Raw TCP transport (byte-for-byte, for CVEs)
│   ├── json.rs           LLM output parsing (handles </think>, fences, noise)
│   ├── payload.rs        Interprets `ip_encoding` and rewrites the payload
│   ├── ip.rs             IPv4 → decimal / hex / octal (pure functions)
│   └── shell.rs          Optional per-lab script execution
├── evaluator/        FlagEvaluator, TimeDelayEvaluator, RegexEvaluator
├── provider/         OllamaProvider, OpenAiCompatibleProvider, TypesafeProvider
├── memory/           SQLite repo: executions, attempts, lessons, lesson_usage
├── labs/
│   ├── spec.rs       YAML schema (LabSpec, ShellSpec)
│   ├── lab.rs        Lab = spec + state; encapsulates everything lab-specific
│   ├── registry.rs   Load all YAMLs from labs/
│   └── runner.rs     Orchestration; owns the global run lock
├── events.rs         Event enum + EngineObserver trait + ConsoleObserver
├── selection.rs      Provider/API-key discovery and menu
└── web/
    ├── state.rs      WebState (live log) + observers
    └── handlers.rs   REST + WebSocket handlers

labs/                YAML lab definitions (edited at runtime from the web panel)
scripts/             Optional helper scripts for labs with `shell:` configured
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
- **`ShellRuntime`** — `process_response(raw) -> (modified, Option<ShellCall>)`. Validates the configured script once at lab load; on each attempt, finds placeholders, executes at most one, and returns the JSON-escaped substitution. Never spawned for labs without a `shell:` block.

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
- [x] **Optional per-lab shell execution** (fixed script, model-issued placeholders)
- [x] **Vulhub integration** — turn real CVEs into YAML labs
- [x] Generalized HTTP executor (headers, raw body, query params, method)
- [ ] Out-of-band evaluator (DNS/HTTP callback)
- [ ] Multi-lab queue (run-all with real sequencing)
- [ ] Anthropic + Gemini providers

---

## License

MIT
