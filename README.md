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
appname = "Check Agent"
sticky = false
open_label = "Open in Zabbix"
notify_acked = false

[app]
max_notif = 5
queue_capacity = 32
rate_limit_max = 5
rate_limit_window = "5s"
```

> ℹ️  Stand-alone Windows builds should keep `appname = ""` (fallback PowerShell AUMID). Once the MSI package registers the custom launcher you can switch to `appname = "Alerting"` to display banners under that name.

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
* Windows packaging flow (custom AppUserModelID):
  1. Build the launcher stub `dotnet publish packaging/windows/AppIdLauncher/AlertingLauncher.csproj -c Release -r win-x64 -o target/launcher --self-contained false`.
  2. Install WiX v4 (one-time): `dotnet tool install --global wix`.
  3. From the repo root, run
     ```powershell
     wix build packaging/msi/alerting.wxs `
       -dAlertingExecutable="C:\\chemin\\vers\\target\\x86_64-pc-windows-msvc\\release\\alerting.exe" `
       -dAlertingLauncherExecutable="C:\\chemin\\vers\\target\\launcher\\AlertingLauncher.exe" `
       -o alerting.msi
     ```
  4. The resulting `alerting.msi` installs both binaries under `%LocalAppData%\Alerting` and provides a Start Menu shortcut that runs the launcher. The launcher applies `AppUserModelID="Alerting"` before spawning `alerting.exe`, so toast banners display under that name.

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
Licensed under the terms described in `LICENSE`. ok
