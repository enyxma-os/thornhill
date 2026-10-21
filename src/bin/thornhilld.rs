use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use thornhill::brain::{Brain, S2Config};
use thornhill::bus::BusServer;
use thornhill::graph::GraphStore;
use thornhill::http::{create_token_file, HttpServer, HttpState};
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

    /// Web konsolunu devre dışı bırak (yalnız UDS)
    #[arg(long)]
    no_http: bool,

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
    info!("👁️  Enyxma Thornhill Daemon v0.1.0 (Faz 6: Ambient Web Console)");
    info!("   PID: {}", std::process::id());
    info!("   Manifesto: Bir uzuv sorulunca çalışır. Thornhill hiç sorulmadığı halde bakar.");
    info!("============================================================");

    // RFC-001 §6: Nedensellik grafiği (SQLite WAL).
    let graph = match GraphStore::open(args.db) {
        Ok(g) => Arc::new(g),
        Err(e) => {
            warn!("Graf deposu açılamadı: {}", e);
            return Err(e);
        }
    };

    // Faz 3: S2 bilişsel katman
    let s2_config = S2Config::from_env();
    info!(
        "🧠 S2 yapılandırması: endpoint={}, model={}, pil modunda izin={}",
        s2_config.endpoint, s2_config.model, s2_config.allow_on_battery
    );
    let brain = Arc::new(Brain::new(Arc::clone(&graph), s2_config));

    let server = Arc::new(BusServer::new(
        args.socket,
        Arc::clone(&graph),
        Arc::clone(&brain),
    ));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Faz 5 — §6.2: Otomatik retention (30 gün; günde bir temizlik)
    let retention_graph = Arc::clone(&graph);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
        loop {
            interval.tick().await;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let thirty_days_ms = 30i64 * 24 * 3600 * 1000;
            if let Ok((ne, nd)) = retention_graph.forget_before(now_ms - thirty_days_ms) {
                if ne > 0 {
                    info!("🧹 Retention: {} olay, {} kenar silindi (30 gün)", ne, nd);
                }
            }
        }
    });

    // Duyu modülleri (Faz 1 Adım 4)
    if !args.no_sensory {
        let (sink_tx, mut sink_rx) = tokio::sync::mpsc::channel(1024);
        let server_for_sensory = Arc::clone(&server);

        tokio::spawn(async move {
            HyprlandSensory::run(sink_tx).await;
        });

        tokio::spawn(async move {
            while let Some(event) = sink_rx.recv().await {
                server_for_sensory.ingest(event);
            }
        });

        info!("Duyu modülü etkin: Hyprland socket2 dinleyicisi başlatıldı");
    } else {
        info!("Duyu modülleri devre dışı (--no-sensory)");
    }

    // ============================================================
    // Faz 6: Ambient Web Console (HTTP + SSE) — UDS protokolü değişmedi;
    // bu katman aynı broadcast kanallarına ve Brain/Registry'ye bağlanır.
    if !args.no_http {
        let token = match create_token_file() {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    "HTTP token oluşturulamadı (web konsolu mutasyonları kapalı): {}",
                    e
                );
                String::new()
            }
        };

        // ADR-062: statik override dizini (varsa sunulur, yoksa include_str!)
        let dashboard_dir = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .ok()
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local/share"))
            })
            .map(|d| d.join("thornhill").join("dashboard"));

        let state = Arc::new(HttpState {
            graph: server.graph(),
            brain: server.brain(),
            registry: server.registry(),
            event_tx: server.event_tx(),
            chip_tx: server.chip_tx(),
            token,
            dashboard_dir,
        });

        tokio::spawn(async move {
            if let Err(e) = HttpServer::run(state).await {
                warn!("Web konsolu hatayla sonlandı: {}", e);
            }
        });
    } else {
        info!("Web konsolu devre dışı (--no-http)");
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
