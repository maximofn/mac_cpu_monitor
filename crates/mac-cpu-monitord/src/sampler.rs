use std::time::{Duration, Instant};

use chrono::Utc;
use mac_cpu_monitor_core::{Cpu, Snapshot};
use tokio::sync::watch;

use crate::source::{ExtraReadings, MacCpuSource};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::source::MacmonAdapter;

/// Build an empty `Cpu` for first-snapshot bootstrap and error fallback.
pub fn empty_cpu(model: Option<String>) -> Cpu {
    Cpu {
        model,
        vendor: None,
        logical_cores: 0,
        physical_cores: None,
        usage_percent: 0.0,
        per_core_usage: Vec::new(),
        temperature_c: None,
        primary_sensor: None,
        temperatures: Vec::new(),
        frequency_mhz: None,
        load_average: None,
        uptime_s: None,
        processes: Vec::new(),
    }
}

pub fn make_snapshot(host: &str, kernel: Option<String>, cpu: Cpu) -> Snapshot {
    Snapshot {
        timestamp: Utc::now().to_rfc3339(),
        host: host.to_string(),
        kernel,
        cpu,
    }
}

/// Spawn a dedicated OS thread that drives `MacCpuSource::sample()` on a
/// fixed cadence and pushes the snapshot through a `watch` channel.
///
/// We use `std::thread` rather than `tokio::spawn` because:
///   1. `MacCpuSource::sample` blocks (sysinfo's two-phase read needs a sleep,
///      and `macmon::get_metrics` blocks for ~1 s on its IOReport channel).
///   2. `macmon::Sampler` is not `Send` — it holds a raw `*const __CFDictionary`
///      via `IOHIDSensors`. Building it inside the spawned thread keeps the
///      whole handle pinned to a single OS thread.
pub fn spawn_sampler(
    mut source: MacCpuSource,
    host: String,
    kernel: Option<String>,
    interval_ms: u64,
    tx: watch::Sender<Snapshot>,
) {
    std::thread::Builder::new()
        .name("cpu-sampler".to_string())
        .spawn(move || {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            let mut macmon = MacmonAdapter::try_init(interval_ms);

            let target = Duration::from_millis(interval_ms.max(200));
            loop {
                let started = Instant::now();

                let extra: ExtraReadings;
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    extra = macmon
                        .as_mut()
                        .map(|m| m.read())
                        .unwrap_or_default();
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    extra = ExtraReadings::default();
                }

                let model_for_fallback = source.cpu_model();
                let cpu = source.sample(extra).unwrap_or_else(|err| {
                    tracing::warn!(error = %err, "CPU sample failed");
                    empty_cpu(model_for_fallback)
                });

                let snap = make_snapshot(&host, kernel.clone(), cpu);
                if tx.send(snap).is_err() {
                    tracing::info!("snapshot channel closed; sampler exiting");
                    break;
                }

                // sample() already burned ~`sample_window` ms inside; only
                // sleep the remainder so the cadence stays close to interval_ms.
                let elapsed = started.elapsed();
                if elapsed < target {
                    std::thread::sleep(target - elapsed);
                }
            }
        })
        .expect("failed to spawn CPU sampler thread");
}
