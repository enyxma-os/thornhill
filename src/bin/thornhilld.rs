use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use thornhill::bus::BusServer;
use thornhill::graph::GraphStore;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::watch;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "thornhilld",
    about = "Enyxma Thornhill Cognitive Daemon (RFC-001)"
)]
struct Args {
    /// Özel UDS soket yolu (varsayılan: $XDG_RUNTIME_DIR/thornhill/thornhill.sock)
    #[arg(short, long)]
    socket: Option<PathBuf>,

    /// Özel graf veritabanı yolu (varsayılan: $XDG_STATE_HOME/thornhill/graph.db)
    #[arg(short, long)]
    db: Option<PathBuf>,

    /// Ayrıntılı loglama (debug)
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    let default_filter = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .init();

    info!("============================================================");
    info!("👁️  Enyxma Thornhill Daemon v0.1.0 (Faz 1, Adım 3: graf dahil)");
    info!("   PID: {}", std::process::id());
    info!("   Manifesto: Bir uzuv sorulunca çalışır. Thornhill hiç sorulmadığı halde bakar.");
    info!("============================================================");

    // RFC-001 §6: Nedensellik grafiği (SQLite WAL). Daemon içinde açılır.
    let graph = match GraphStore::open(args.db) {
        Ok(g) => Arc::new(g),
        Err(e) => {
            warn!("Graf deposu açılamadı: {}", e);
            return Err(e);
        }
    };

    let server = Arc::new(BusServer::new(args.socket, graph));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // SIGINT ve SIGTERM yakalama
    tokio::spawn(async move {
        let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT dinlenemedi");
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM dinlenemedi");

        tokio::select! {
            _ = sigint.recv() => {
                info!("SIGINT sinyali alındı.");
            }
            _ = sigterm.recv() => {
                info!("SIGTERM sinyali alındı.");
            }
        }

        let _ = shutdown_tx.send(true);
    });

    if let Err(e) = server.run(shutdown_rx).await {
        warn!("Thornhill daemon hatayla sonlandı: {}", e);
    }

    info!("Thornhill daemon temiz bir şekilde kapandı.");
    Ok(())
}
