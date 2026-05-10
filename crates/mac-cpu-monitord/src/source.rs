use std::time::Duration;

use anyhow::Result;
use mac_cpu_monitor_core::{Cpu, LoadAverage, Process, TempSensor};
use sysinfo::{CpuRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

/// Combined macOS CPU sampler.
///
/// `sysinfo` does the cross-platform heavy lifting: usage %, per-core usage,
/// processes, and on Intel also frequency. On Apple Silicon `sysinfo`'s
/// frequency is always 0 and `Components` is empty, so we layer `macmon` on top
/// to read CPU temperature and cluster (E/P) frequency via the private
/// IOReport framework — same data source as `powermetrics`, but without sudo.
///
/// `MacCpuSource` is `Send` (just owns sysinfo state) but the `macmon::Sampler`
/// it pairs with is **not** (`IOHIDSensors` holds raw `*const __CFDictionary`).
/// We therefore initialise macmon *inside* the sampling thread rather than
/// keeping it as a field — see `crate::sampler::spawn_sampler`.
pub struct MacCpuSource {
    sys: System,
    top_processes: usize,
    cpu_model: Option<String>,
    cpu_vendor: Option<String>,
    physical_cores: Option<u32>,
    /// `sysinfo` needs a non-trivial gap between two `refresh_cpu_usage` calls
    /// for the busy/total deltas to be non-zero. We measure usage over this
    /// window inside `sample()`.
    sample_window: Duration,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub struct MacmonAdapter {
    sampler: macmon::Sampler,
    /// Microseconds of the IOReport sampling window. macmon::get_metrics
    /// blocks for `duration` ms and divides it across 4 sub-samples, so this
    /// value also dominates each `sample()` call's wall time on aarch64.
    duration_ms: u32,
}

/// Auxiliary readings layered onto the sysinfo sample. `sample()` ignores it
/// when `None` (Intel Mac, or macmon init failed).
#[derive(Default)]
pub struct ExtraReadings {
    pub temperature_c: Option<f32>,
    pub primary_sensor: Option<String>,
    pub temperatures: Vec<TempSensor>,
    pub frequency_mhz: Option<f32>,
}

impl MacCpuSource {
    pub fn new(top_processes: usize, sample_interval_ms: u64) -> Self {
        let refresh = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_processes(ProcessRefreshKind::nothing().with_cpu().with_memory());
        let mut sys = System::new_with_specifics(refresh);
        sys.refresh_cpu_all();

        let cpu_model = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .filter(|s| !s.is_empty());
        let cpu_vendor = sys
            .cpus()
            .first()
            .map(|c| c.vendor_id().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| Some("Apple".to_string()));
        let physical_cores = System::physical_core_count().map(|c| c as u32);

        // Anything shorter than ~200 ms underflows sysinfo's per-core deltas;
        // anything longer than 900 ms wastes the macmon window we layer on top.
        let sample_window = Duration::from_millis(sample_interval_ms.clamp(200, 900));

        Self {
            sys,
            top_processes,
            cpu_model,
            cpu_vendor,
            physical_cores,
            sample_window,
        }
    }

    pub fn cpu_model(&self) -> Option<String> {
        self.cpu_model.clone()
    }

    /// Collect a fresh snapshot. Blocks for at least `sample_window` ms inside
    /// (sysinfo's two-phase read). Call only from the dedicated sampler thread.
    pub fn sample(&mut self, extra: ExtraReadings) -> Result<Cpu> {
        // Refresh once, sleep, refresh again — sysinfo's per-core usage is the
        // delta between successive snapshots of host_processor_info() ticks.
        self.sys.refresh_cpu_usage();
        std::thread::sleep(self.sample_window);
        self.sys.refresh_cpu_usage();
        self.sys
            .refresh_processes(ProcessesToUpdate::All, true);

        let usage_percent = self.sys.global_cpu_usage();
        let per_core_usage: Vec<f32> = self.sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        let logical_cores = per_core_usage.len() as u32;

        // sysinfo Cpu::frequency() returns u64 MHz; on Apple Silicon it's 0,
        // so prefer the macmon-derived cluster average when available.
        let sysinfo_freq: Option<f32> = self
            .sys
            .cpus()
            .first()
            .map(|c| c.frequency())
            .filter(|f| *f > 0)
            .map(|f| f as f32);
        let frequency_mhz = extra.frequency_mhz.or(sysinfo_freq);

        let load_average = Some({
            let l = System::load_average();
            LoadAverage {
                one: l.one as f32,
                five: l.five as f32,
                fifteen: l.fifteen as f32,
            }
        });
        let uptime_s = Some(System::uptime());

        let processes = self.collect_top_processes();

        Ok(Cpu {
            model: self.cpu_model.clone(),
            vendor: self.cpu_vendor.clone(),
            logical_cores,
            physical_cores: self.physical_cores,
            usage_percent,
            per_core_usage,
            temperature_c: extra.temperature_c,
            primary_sensor: extra.primary_sensor,
            temperatures: extra.temperatures,
            frequency_mhz,
            load_average,
            uptime_s,
            processes,
        })
    }

    fn collect_top_processes(&self) -> Vec<Process> {
        if self.top_processes == 0 {
            return Vec::new();
        }
        let mut all: Vec<&sysinfo::Process> = self.sys.processes().values().collect();
        all.sort_by(|a, b| {
            b.cpu_usage()
                .partial_cmp(&a.cpu_usage())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all.into_iter()
            .take(self.top_processes)
            .map(|p| Process {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: p.cpu_usage(),
                memory_bytes: p.memory(),
            })
            .collect()
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
impl MacmonAdapter {
    pub fn try_init(sample_interval_ms: u64) -> Option<Self> {
        match macmon::Sampler::new() {
            Ok(sampler) => {
                let duration_ms = sample_interval_ms.clamp(400, 2_000) as u32;
                Some(Self { sampler, duration_ms })
            }
            Err(err) => {
                tracing::warn!(error = %err, "macmon init failed; temperature/freq will be null");
                None
            }
        }
    }

    pub fn read(&mut self) -> ExtraReadings {
        match self.sampler.get_metrics(self.duration_ms) {
            Ok(m) => {
                let mut extra = ExtraReadings::default();
                if m.temp.cpu_temp_avg > 0.0 {
                    extra.temperature_c = Some(m.temp.cpu_temp_avg);
                    extra.primary_sensor = Some("CPU (IOReport avg)".to_string());
                    extra.temperatures.push(TempSensor {
                        chip: "SoC".to_string(),
                        label: "CPU avg".to_string(),
                        temp_c: m.temp.cpu_temp_avg,
                    });
                    if m.temp.gpu_temp_avg > 0.0 {
                        extra.temperatures.push(TempSensor {
                            chip: "SoC".to_string(),
                            label: "GPU avg".to_string(),
                            temp_c: m.temp.gpu_temp_avg,
                        });
                    }
                }

                // macmon exposes (frequency_mhz, percent_from_max) per CPU
                // cluster. Apple Silicon doesn't publish a single "package"
                // frequency, so we report the average of the P and E clusters
                // weighted by core count.
                let soc = self.sampler.get_soc_info();
                let pcores = soc.pcpu_cores as f32;
                let ecores = soc.ecpu_cores as f32;
                let pfreq = m.pcpu_usage.0 as f32;
                let efreq = m.ecpu_usage.0 as f32;
                let total = pcores + ecores;
                if total > 0.0 && (pfreq > 0.0 || efreq > 0.0) {
                    extra.frequency_mhz = Some((pfreq * pcores + efreq * ecores) / total);
                }
                extra
            }
            Err(err) => {
                tracing::debug!(error = %err, "macmon sample failed");
                ExtraReadings::default()
            }
        }
    }
}

/// Build a "kernel" string roughly equivalent to `uname -sr`.
pub fn read_kernel_version() -> Option<String> {
    let kind = sysinfo::System::kernel_version();
    let osname = sysinfo::System::name();
    match (osname, kind) {
        (Some(n), Some(v)) => Some(format!("{n} {v}")),
        (None, Some(v)) => Some(v),
        _ => None,
    }
}
