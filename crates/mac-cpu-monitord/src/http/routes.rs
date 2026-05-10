use axum::extract::State;
use axum::Json;
use mac_cpu_monitor_core::{Cpu, Process, Snapshot, TempSensor};
use serde::Serialize;

use super::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub uptime_s: u64,
}

pub async fn healthz(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        uptime_s: state.started_at.elapsed().as_secs(),
    })
}

#[derive(Serialize)]
pub struct InfoResponse {
    pub backend_version: &'static str,
    pub api_version: &'static str,
    pub host: String,
    pub kernel: Option<String>,
    pub cpu_model: Option<String>,
    pub logical_cores: u32,
    pub physical_cores: Option<u32>,
}

pub async fn info(State(state): State<AppState>) -> Json<InfoResponse> {
    let snap = state.snapshot_rx.borrow();
    Json(InfoResponse {
        backend_version: env!("CARGO_PKG_VERSION"),
        api_version: mac_cpu_monitor_core::API_VERSION,
        host: snap.host.clone(),
        kernel: snap.kernel.clone(),
        cpu_model: snap.cpu.model.clone(),
        logical_cores: snap.cpu.logical_cores,
        physical_cores: snap.cpu.physical_cores,
    })
}

pub async fn snapshot(State(state): State<AppState>) -> Json<Snapshot> {
    Json(state.snapshot_rx.borrow().clone())
}

pub async fn cpu(State(state): State<AppState>) -> Json<Cpu> {
    Json(state.snapshot_rx.borrow().cpu.clone())
}

pub async fn temperatures(State(state): State<AppState>) -> Json<Vec<TempSensor>> {
    Json(state.snapshot_rx.borrow().cpu.temperatures.clone())
}

pub async fn processes(State(state): State<AppState>) -> Json<Vec<Process>> {
    Json(state.snapshot_rx.borrow().cpu.processes.clone())
}
