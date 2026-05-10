mod config;
mod http;
mod sampler;
mod source;

use std::net::SocketAddr;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use config::Config;
use sampler::{empty_cpu, make_snapshot, spawn_sampler};
use source::{read_kernel_version, MacCpuSource};
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::parse();
    init_tracing(&cfg.log_level);

    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());
    let kernel = read_kernel_version();

    let source = MacCpuSource::new(cfg.top_processes as usize, cfg.sample_interval_ms);

    // Bootstrap snapshot so /v1/snapshot returns something before the first
    // tick lands. Only the cpu_model is filled — the rest stays at zeros.
    let initial = make_snapshot(&host, kernel.clone(), empty_cpu(source.cpu_model()));
    let (tx, rx) = watch::channel(initial);

    spawn_sampler(source, host.clone(), kernel, cfg.sample_interval_ms, tx);

    let state = http::AppState {
        started_at: Instant::now(),
        snapshot_rx: rx,
    };
    let app = http::build_router(state);

    let addr = SocketAddr::new(cfg.bind, cfg.port);
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    tracing::info!(%addr, "mac-cpu-monitord listening");

    tokio::select! {
        result = axum::serve(listener, app) => {
            result.context("HTTP server error")?;
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown requested; aborting in-flight SSE streams");
        }
    }

    tracing::info!("shutdown complete");
    Ok(())
}

fn init_tracing(directive: &str) {
    let filter = EnvFilter::try_new(directive).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "could not install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("ctrl-c received"),
        _ = terminate => tracing::info!("SIGTERM received"),
    }
}
