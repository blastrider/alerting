# Alerting

Zabbix JSON-RPC client that turns active problems into safe, rate-limited desktop notifications with structured tracing and hardened systemd packaging.

## Getting Started
1. `cargo install --path . --locked`
2. `mkdir -p ~/.config/alerting` and drop a copy of `examples/config.toml` there
3. Export `ZBX_TOKEN` or add it to the config file (never commit it)
4. Run `alerting --config ~/.config/alerting/config.toml --once` to validate credentials
5. Optional: `just ci` to check fmt/clippy/tests/audit locally
6. Install the user service: `systemctl --user enable --now alerting.service` (see packaging section)

## Configuration
The loader merges **defaults < file < environment**. All durations accept [humantime](https://docs.rs/humantime) strings (`30s`, `5m` …).

```toml
# ~/.config/alerting/config.toml
[zabbix]
url = "https://monitoring.example.com/api_jsonrpc.php"
limit = 25
concurrency = 6
ack_filter = "unack"

[notify]
appname = ""
sticky = false
open_label = "Open in Zabbix"
notify_acked = false

[app]
max_notif = 5
queue_capacity = 32
rate_limit_max = 5
rate_limit_window = "5s"
```

### Environment overrides
| Variable | Description | Default |
| --- | --- | --- |
| `CONFIG_FILE` | Alternative config path | `config.toml` in cwd |
| `ZBX_URL` | JSON-RPC endpoint | config value |
| `ZBX_TOKEN` | API token (required) | — |
| `LIMIT` | Max problems fetched per poll | `limit` field |
| `CONCURRENCY` | Parallel host lookups | `concurrency` field |
| `ACK_FILTER` | `ack`, `unack`, or `all` | `ack_filter` |
| `MAX_NOTIF` | Cap notifications per loop (1..=100) | `max_notif` |
| `NOTIFY_STICKY` | Make toasts persistent | `sticky` |
| `POLL_INTERVAL` | Interval between polls | `poll_interval` |
| `RATE_LIMIT_MAX` / `_WINDOW` | Leaky bucket budget | see file |

### Telemetry
Tracing uses `RUST_LOG` (default `info`). `--json-logs` switches to JSON formatting when the binary is built with the `json-logs` feature.

### CLI
```
USAGE: alerting [FLAGS]
    --config <PATH>      # Config file override (default: config.toml)
    --interval <DUR>     # Override poll interval (humantime)
    --max-notif <N>      # Limit notifications per loop (1..=100)
    --once               # Single poll, then exit
    --dry-run            # Log queue entries, skip desktop notifications
    --insecure           # Allow plain HTTP endpoints (⚠️ only on trusted networks)
    --json-logs          # Enable JSON tracing layout when compiled with json-logs
```
Each request is tagged with a correlation id header (`x-correlation-id`) and logged along with event id, host, severity and queue latency.

## Scheduling & Packaging
* Hardened user service at `packaging/systemd/user/alerting.service` – install via `systemctl --user enable --now alerting`.
* `.deb` metadata ready for [`cargo-deb`](https://github.com/mmstick/cargo-deb): `cargo deb` produces a package shipping the binary and the user unit under `/usr/share/doc/alerting`.
* Windows MSI template (`packaging/msi/alerting.wxs`) targets per-user installs with fixed GUIDs; provide `AlertingExecutable` to `candle`/`light`.

## Security Notes
* Store `ZBX_TOKEN` outside Git, ideally via an environment file (`chmod 600`).
* By default HTTPS is enforced; `--insecure` and HTTP URLs are rejected unless explicitly allowed.
* Notifications suppresss secrets in logs (`SecretString`).

## Troubleshooting
| Symptom | Check |
| --- | --- |
| `5xx` errors | Zabbix maintenance, missing proxy headers, or rate limits – inspect structured logs with the correlation id |
| `timeout while fetching` | Increase `poll_interval`/`limit`, verify outbound connectivity, ensure system clock is correct |
| Proxy in path | Set `HTTPS_PROXY`/`NO_PROXY` before launching the service |
| Empty toasts | Enable `RUST_LOG=debug` to inspect payloads and confirm `ack_filter` |

## Testing
`just ci` wraps `cargo fmt`, `cargo clippy`, `cargo nextest`, `cargo deny`, `cargo audit`, and `cargo geiger`. Integration tests spawn local mock servers; when sandboxed, grant permission to bind loopback sockets (`cargo test` with escalated permissions in the CI workflow).

## License
Licensed under the terms described in `LICENSE`.
