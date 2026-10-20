use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use thornhill::bus::BusServer;
use thornhill::graph::GraphStore;
use thornhill::sensory::hyprland::HyprlandSensory;
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

    /// Duyu modüllerini devre dışı bırak (yalnız bus + graf; test için)
    #[arg(long)]
    no_sensory: bool,

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
    info!("👁️  Enyxma Thornhill Daemon v0.1.0 (Faz 1, Adım 4: Hyprland duyu)");
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

    // Duyu modülleri (Faz 1 Adım 4): Hyprland socket2 -> ingest()
    if !args.no_sensory {
        let (sink_tx, mut sink_rx) = tokio::sync::mpsc::unbounded_channel();
        let server_for_sensory = Arc::clone(&server);

        // Duyu üreticisi: socket2 satırlarını kanonik olaylara çevirir.
        tokio::spawn(async move {
            HyprlandSensory::run(sink_tx).await;
        });

        // Duyu tüketicisi: olayları bus'a (grafe + yayın) aktarır.
        tokio::spawn(async move {
            while let Some(event) = sink_rx.recv().await {
                server_for_sensory.ingest(event);
            }
        });

        info!("Duyu modülü etkin: Hyprland socket2 dinleyicisi başlatıldı");
    } else {
        info!("Duyu modülleri devre dışı (--no-sensory)");
    }

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
