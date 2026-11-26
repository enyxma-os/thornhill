# Enyxma Thornhill

> **Language / Dil:** **English** | [Türkçe](./README.tr.md)

A local Rust service for Linux desktops that observes system events, builds a causal graph across events, and presents actionable approval-gated chips to the user.

- Repository: https://codeberg.org/enyxma/thornhill
- License: GPL-3.0-or-later — [LICENSE](./LICENSE)
- Language: Rust with asynchronous Tokio runtime
- Binaries: `thornhilld` (daemon) and `thornhill-ctl` (CLI client)
- Packaging: Nix Flake exposing `packages.default`, `nixosModules.default`, and `homeManagerModules.default`

## Architecture

```text
+------------------------------+      +---------------------------------+
| Sensory Tap                  |      | Causal Graph                    |
| Hyprland socket2 stream      |----->| SQLite WAL                      |
| shell hooks: fish, zsh, bash |event | tables: events, edges,          |
|                              |      | lessons, habits                 |
+------------------------------+      +----------------+----------------+
                                                       |
                                        +--------------v---------------+
                                        | S1 Reflex Engine             |
                                        | deterministic rules          |
                                        | target: < 1 ms, 0 VRAM       |
                                        +--------------+---------------+
                                                       | Action Chip
+------------------------------+      +----------------v---------------+
| S2 Cognitive Layer           |      | Action Executor                |
| local Ollama query           |<-----| approval ladder & TTL audit    |
| rate limit, battery lock     |      | ledger                         |
+------------------------------+      +--------------------------------+
       UNIX Domain Socket: $XDG_RUNTIME_DIR/thornhill/thornhill.sock (NDJSON)
+------------------------------+      +---------------------------------+
| Quickshell QML Layer         |      | Web Console                     |
| shell/ directory             |      | http://127.0.0.1:9099           |
+------------------------------+      +---------------------------------+
```

### Sensory Tap

- Reads Hyprland's `$XDG_RUNTIME_DIR/hypr/<HIS>/.socket2.sock` event stream. Session discovery uses `HYPRLAND_INSTANCE_SIGNATURE`, falling back to scanning `hypr/` for `.socket2.sock` without hardcoded paths.
- Shell hooks live in `shell-hook/thornhill.sh` and `shell-hook/thornhill.fish`. Only non-zero exit codes are reported; hooks exit silently when the binary or socket is absent.
- Fully event-driven with exponential backoff reconnection and zero polling loops.

### Causal Graph

- Database path: `$XDG_STATE_HOME/thornhill/graph.db` in SQLite WAL mode.
- `events` stores canonical events; `edges` tracks relations such as `caused` with support and confidence scores. Repeated patterns (3+ occurrences) promote into persistent summaries in `lessons`.
- `forget` prunes events and linked edges older than a cutoff while preserving `lessons` and `habits`.

### S1 Reflex Engine

- Zero LLM calls, zero VRAM usage. Produces Action Chips via deterministic rules on known triggers (such as failed commands) targeting sub-millisecond latency.
- Each Action Chip carries structured actions with labels and risk classifications.

### S2 Cognitive Layer

- Activated only on explicit user queries or when clicking an explanation action on a chip.
- Packages recent graph events, edges, and learned lessons into a single request to the local Ollama API, enforcing a structured JSON schema (`explanation`, `remedy`).
- Rate-limited (2s for user queries, 10s for chip-triggered calls; timestamp recorded only after success) and locked by default on battery power.

### Desktop & Web Interfaces

- QML components under `shell/` render Action Chips directly on the desktop via Quickshell.
- The Web Console is an embedded HTTP + SSE server inside `thornhilld` (default `http://127.0.0.1:9099`, configurable via `THORNHILL_HTTP_ADDR`).
- Includes a D3.js force-directed causal graph, event timeline, live event stream, Action Chip list, S2 query console, and right-to-forget controls.

## Quick Start

Build, test, and run from source:

```bash
git clone https://codeberg.org/enyxma/thornhill.git
cd thornhill
nix develop -c cargo test
nix develop -c just verify
nix build .#packages.x86_64-linux.default --no-link
./result/bin/thornhilld --verbose
```

## CLI Usage

```bash
# Daemon status and capabilities
thornhill-ctl status

# Graph telemetry: events, edges, lessons, and habits counts
thornhill-ctl stats

# Inject a synthetic event to trigger the S1 reflex engine
thornhill-ctl inject --src shell --kind cmd.done --subject test \
  --ctx '{"exit_code": 101, "cmd": "cargo build", "error": "E0432"}'

# Live event and chip stream
thornhill-ctl listen

# Structured graph query: recent, causal, lessons
thornhill-ctl query -q causal

# Query the S2 cognitive layer
thornhill-ctl ask "explain the cause of the last build error"

# Right to forget: prune events older than cutoff (default 30d)
thornhill-ctl forget --before 7d

# Chip action: preview first, then confirm
thornhill-ctl chip-act chip_01M5DMB2M88REZC5R6F181SBY1
thornhill-ctl chip-act chip_01M5DMB2M88REZC5R6F181SBY1 --confirm
```

## Web Console

Default address: `http://127.0.0.1:9099` (configurable via `THORNHILL_HTTP_ADDR`; non-loopback binding requires `THORNHILL_HTTP_ALLOW_REMOTE=1`).

| Path | Method | Purpose |
|---|---|---|
| `/` | GET | Dashboard |
| `/api/stream` | GET | Live SSE event and chip stream |
| `/api/stats` | GET | Graph counters |
| `/api/query` | GET | Query `recent`, `causal`, `lessons` |
| `/api/ask` | POST | S2 query |
| `/api/forget` | POST | Prune old events |
| `/api/chip-act` | POST | Execute chip action |

## Security & Approval Ladder

- The UDS socket uses `0700` permissions and `SO_PEERCRED` peer UID verification.
- The Web Console binds to loopback only and enforces `Host`, `Origin`, and `X-Thornhill-Token` (written with `0600` permissions to `$XDG_RUNTIME_DIR/thornhill/http.token`) on mutating routes.
- Action risk classes:

| Risk | Behavior |
|---|---|
| `none` | Executed on single click |
| `low` | Runs dry-run validation only; no mutation |
| `medium` | Two-step approval: preview followed by hold-to-confirm |
| `high` | Never executed; presented as advice only |

- Data stays local by default; S2 queries go only to the configured endpoint (local Ollama by default).
