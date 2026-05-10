# Mac CPU Monitor

Real-time CPU monitor for macOS. Split into a small backend daemon that samples `host_processor_info()` + IOReport (via [`sysinfo`](https://crates.io/crates/sysinfo) and [`macmon`](https://crates.io/crates/macmon)) and exposes an HTTP/SSE API, plus a native Swift menubar frontend that renders an icon (CPU silhouette + temperature label + usage-percent donut) into `NSStatusItem`.

Same on-the-wire schema as the Linux [`cpu_monitor`](../cpu_monitor) sibling, just a different backend and a different port — both can run side by side on the same Mac.

## Architecture

```
+-------------------------+        HTTP/SSE         +----------------------------+
|   mac-cpu-monitord      | <---------------------- |   Mac CPU Monitor.app      |
|  (sysinfo + macmon)     |    /v1/stream JSON      |  (NSStatusItem + AppKit)   |
+-------------------------+                         +----------------------------+
        ^                                                       ^
        | host_processor_info() / sysctl                        | NSStatusBar
        | macmon → IOReport (cpu temp, cluster freq)            v
        v                                                  macOS menu bar
   XNU kernel
```

The Rust binaries live in a single Cargo workspace under `crates/`:

- `mac-cpu-monitor-core` — shared `Snapshot` / `Cpu` / `Process` / `TempSensor` types serialised with `serde`. Identical to the Linux backend's schema so external consumers (Home Assistant, dashboards, etc.) work against either backend unchanged.
- `mac-cpu-monitord` — backend daemon. Uses `sysinfo` for usage / per-core / processes / load avg / uptime / model, and on Apple Silicon layers `macmon` on top for CPU temperature and per-cluster frequency via the private IOReport framework — same data source as `powermetrics`, but without `sudo`. Defaults to `127.0.0.1:9125`.

The macOS frontend lives in `front-mac/` as a Swift Package (Swift + AppKit + CoreGraphics, zero third-party deps). It consumes `/v1/stream` and renders into the menubar via `NSStatusItem`.

## Requirements

- macOS 13 or later (the Swift package targets `.macOS(.v13)`).
- Apple Silicon (`arm64`) or Intel (`x86_64`).
- **Rust toolchain ≥ 1.85** (stable). Install via [rustup](https://rustup.rs):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  source "$HOME/.cargo/env"
  ```
- **Swift 5.9+** (Xcode Command Line Tools): `xcode-select --install`.

`macmon` is `aarch64-apple-darwin`-only. On Intel Macs the backend still builds and runs — `temperature_c` and the cluster-derived `frequency_mhz` come back `null`, the rest works through `sysinfo`. To add SMC-based temperature reading on Intel, plug in [`macsmc`](https://crates.io/crates/macsmc) under a `cfg(target_arch = "x86_64")` block in `crates/mac-cpu-monitord/Cargo.toml`.

No `sudo` is required at runtime: every data source used (sysinfo, IOReport via macmon) works in user space.

## Build

```bash
# Backend
cargo build --release --workspace
# → target/release/mac-cpu-monitord

# Frontend
cd front-mac
./scripts/build-app.sh
# → front-mac/build/Mac CPU Monitor.app
```

## Run

In two terminals (or as services — see Autostart below):

```bash
./target/release/mac-cpu-monitord --bind 127.0.0.1 --port 9125
open "front-mac/build/Mac CPU Monitor.app"
```

Or pass a custom backend URL explicitly:

```bash
"front-mac/build/Mac CPU Monitor.app/Contents/MacOS/mac-cpu-monitor-tray" \
    --backend-url http://127.0.0.1:9125
```

### Daemon flags

| Flag | Default | Purpose |
|---|---|---|
| `--bind` | `127.0.0.1` | bind address (no auth, keep loopback) |
| `--port` | `9125` | HTTP port (Linux backend uses `9124` — different on purpose) |
| `--sample-interval-ms` | `1000` | sampler period |
| `--top-processes` | `5` | top-N CPU consumers per snapshot (`0` disables) |
| `--log-level` | `info` | also via `RUST_LOG` |

### Tray flags

`--backend-url`, `--icon-height`, `--dump-icon <path>` (renders one snapshot to PNG and exits — useful to inspect what the menubar receives without fighting AppKit), `--version`, `-h`.

### Quick API smoke test

```bash
curl -s http://127.0.0.1:9125/v1/snapshot | jq
curl -N http://127.0.0.1:9125/v1/stream         # SSE: one event per second
```

## API

| Endpoint | Purpose |
|---|---|
| `GET /healthz` | liveness |
| `GET /v1/info` | backend / kernel / cpu_model metadata |
| `GET /v1/snapshot` | full latest snapshot |
| `GET /v1/cpu` | just the `cpu` object (usage, per-core, temps, freq, load, processes) |
| `GET /v1/cpu/temperatures` | sensor list |
| `GET /v1/cpu/processes` | top processes |
| `GET /v1/stream` | SSE — one snapshot per event |

## Autostart on login

Two LaunchAgents live in `front-mac/scripts/`. Run from `front-mac/`:

```bash
./scripts/install-daemon.sh         # backend on login (port 9125, KeepAlive)
./scripts/install-launchagent.sh    # tray autostart on login
```

Logs land in `~/Library/Logs/mac-cpu-monitord.{out,err}.log` and `~/Library/Logs/mac-cpu-monitor-tray.{out,err}.log`. Pass `uninstall` to either script to remove its agent.

## Notes on the data sources

- **CPU temperature** on Apple Silicon comes from macmon's `TempMetrics::cpu_temp_avg`, which averages PECI / `Tp…` / `Te…` SMC keys (or HID `pACC/eACC MTR Temp Sensor` on older macOS). Reported as a single value labelled `SoC / CPU avg` in `temperatures[]`; the GPU-cluster average is also exposed when non-zero.
- **CPU frequency** on Apple Silicon is the IOReport per-cluster average weighted by core count: `(P_freq × P_cores + E_freq × E_cores) / total`. Apple does not publish a single package frequency.
- **Per-core usage and global usage** come from `sysinfo`, which on macOS wraps `host_processor_info()`.
- **Load average and uptime** come from `sysinfo`'s static methods (`getloadavg(3)` and `kern.boottime` under the hood).

## License

MIT — see `LICENSE`.
