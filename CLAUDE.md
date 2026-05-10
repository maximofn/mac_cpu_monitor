# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project layout

Two halves living side by side, sharing nothing at runtime except the JSON wire-format:

- **Rust backend** in `crates/` (Cargo workspace): `mac-cpu-monitor-core` (shared serde types) + `mac-cpu-monitord` (HTTP/SSE daemon, default `127.0.0.1:9125`).
- **Swift frontend** in `front-mac/` (Swift Package, AppKit, no third-party deps): a menubar-only app (`LSUIElement`) that consumes the daemon's `/v1/stream`.

The on-the-wire schema (`crates/mac-cpu-monitor-core/src/model.rs` ↔ `front-mac/Sources/MacCPUMonitorTray/Models.swift`) is intentionally identical to the Linux sibling at `../cpu_monitor`. **If you add or rename a field on the Rust side, mirror it in `Models.swift` (with the matching `CodingKeys`) or the JSON decode silently drops it.**

## Common commands

Rust requires `rustup` (≥ 1.85, edition 2024 deps). It was installed with `--no-modify-path`, so prefix Rust commands with `. "$HOME/.cargo/env"` if `cargo` isn't already on `PATH`.

```bash
# Rust (run from repo root)
cargo build --workspace                 # debug
cargo build --release --workspace       # release → target/release/mac-cpu-monitord
cargo test --workspace                  # core has a JSON-roundtrip test
cargo clippy --workspace --all-targets
cargo test -p mac-cpu-monitor-core model::tests::snapshot_roundtrips_through_json

# Swift (run from front-mac/)
./scripts/build-app.sh                  # → build/Mac CPU Monitor.app
swift build -c release --arch arm64     # raw binary only, no .app wrapper

# End-to-end
./target/release/mac-cpu-monitord --port 9125 &
open "front-mac/build/Mac CPU Monitor.app"
curl -s http://127.0.0.1:9125/v1/snapshot | jq
curl -N http://127.0.0.1:9125/v1/stream      # SSE, one event per second
"front-mac/build/Mac CPU Monitor.app/Contents/MacOS/mac-cpu-monitor-tray" --dump-icon /tmp/icon.png
```

## Architecture notes that span files

### Sampling thread model (Rust)

`crates/mac-cpu-monitord/src/sampler.rs::spawn_sampler` runs on a **dedicated `std::thread`**, not on the tokio runtime. Two reasons, both load-bearing:

1. `MacCpuSource::sample` blocks: sysinfo's two-phase counter read needs a `std::thread::sleep` between refreshes, and on Apple Silicon `macmon::Sampler::get_metrics(duration_ms)` blocks for that full duration on its IOReport channel.
2. `macmon::Sampler` is **not `Send`** — it holds a raw `*const __CFDictionary` via `IOHIDSensors`. Initialising it inside the spawned closure pins the handle to one OS thread. Don't try to share it via `Arc<Mutex<…>>` or move it across `tokio::spawn` — it won't compile and the previous attempt to do so was reverted.

The sampler thread owns `MacCpuSource` outright and pushes snapshots to the HTTP layer through a `tokio::sync::watch` channel (whose `send` is sync-friendly).

### Two data sources, one snapshot

`MacCpuSource` (`source.rs`) wraps `sysinfo` for the cross-platform metrics. On Apple Silicon, `MacmonAdapter` (also in `source.rs`, gated by `cfg(all(target_os = "macos", target_arch = "aarch64"))`) layers temperature + cluster frequency on top via `macmon`. The frontend gets a single `Cpu` struct; `frequency_mhz` prefers the macmon-derived P/E cluster average weighted by core count, falling back to `sysinfo::Cpu::frequency()` (which is `0` on Apple Silicon and meaningful on Intel).

On Intel Macs the `macmon` dependency is not compiled in, so `temperature_c`, `temperatures`, and the cluster-derived `frequency_mhz` come back `null`. To add SMC-based temps on Intel, add [`macsmc`](https://crates.io/crates/macsmc) under a `cfg(target_arch = "x86_64")` block in `crates/mac-cpu-monitord/Cargo.toml` and a parallel adapter.

### HTTP/SSE surface

`crates/mac-cpu-monitord/src/http/mod.rs` wires the routes; routes only ever read the latest `Snapshot` from the `watch::Receiver` clone in `AppState`. SSE (`http/sse.rs`) wraps that receiver in `tokio_stream::wrappers::WatchStream` so each new snapshot becomes one SSE event automatically — there is no per-client buffering or sample loop on the HTTP side.

Endpoints: `/healthz`, `/v1/info`, `/v1/snapshot`, `/v1/cpu`, `/v1/cpu/temperatures`, `/v1/cpu/processes`, `/v1/stream`. Defaults to `127.0.0.1:9125`. Port `9125` is deliberate: the Linux backend at `../cpu_monitor` uses `9124`, so both can run side-by-side (e.g. when tunnelling a remote Linux box's CPU into the same Mac that runs this local backend).

### Swift menubar app

`StatusBarController.refreshIcon` dedupes via a render key (`pct:temp|connected|appearance`) so identical 1-Hz ticks don't repaint. Light/dark switching listens on `AppleInterfaceThemeChangedNotification` via `DistributedNotificationCenter` — **don't KVO `effectiveAppearance` on the status item button**, the comment in that file explains the feedback loop that caused.

`SSEClient` (`Client.swift`) parses SSE manually because `Foundation.AsyncBytes.lines` collapses the blank-line frame separators; it decodes a `Snapshot` after every `data:` line on the assumption that `mac-cpu-monitord` ships one self-contained JSON snapshot per event (which it does — see `http/sse.rs`).

`IconRenderer` is the only file copied verbatim from the sibling Linux project's `front-mac/`. The base icon (`Resources/cpu.png`) is loaded via `Bundle.module`; `build-app.sh` copies the SwiftPM-generated resource bundle next to the binary inside the `.app/Contents/MacOS/` so `Bundle.module` resolves at runtime.

### Autostart

`front-mac/scripts/install-daemon.sh` and `install-launchagent.sh` install two LaunchAgents under `~/Library/LaunchAgents/`. The plists hardcode the absolute path to `target/release/mac-cpu-monitord` and to the bundled `.app`; if the project moves on disk, regenerate them or run the install scripts again.

## When changing the schema

1. Edit `crates/mac-cpu-monitor-core/src/model.rs`.
2. Mirror in `front-mac/Sources/MacCPUMonitorTray/Models.swift` — same field order, matching `CodingKeys` for the snake_case ↔ camelCase mapping.
3. Rebuild both halves: `cargo build --workspace` and `./front-mac/scripts/build-app.sh`.
4. Smoke test: `curl -s http://127.0.0.1:9125/v1/snapshot | jq` to confirm new fields serialise; the Swift side will silently ignore unknown JSON keys, so the failure mode is "field stays `nil`/`zero`" — easy to miss without an end-to-end check.

The same schema is used by the Linux `cpu-monitord` at `../cpu_monitor`; keep them in sync if the change is supposed to be cross-platform (e.g. so a single Home Assistant package works against both backends).
