use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Instant;
use thornhill::bus::BusClient;
use thornhill::protocol::{CanonicalEvent, SubFilter, WireRequest, WireResponse};

#[derive(Parser, Debug)]
#[command(
    name = "thornhill-ctl",
    about = "Enyxma Thornhill Kontrol ve Enjeksiyon Aracı"
)]
struct Cli {
    /// Özel UDS soket yolu
    #[arg(short, long)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Daemon canlılık yoklaması ve gecikme ölçümü
    Ping,

    /// Daemon durum ve özelliklerini listele
    Status,

    /// Sentetik kanonik olay enjekte et (Test ve S1 doğrulaması için)
    Inject {
        /// Olay kaynağı (shell, hyprland, git, udev, user vb.)
        #[arg(short, long, default_value = "shell")]
        src: String,

        /// Olay türü (örn: cmd.done, win.focus, repo.enter)
        #[arg(short, long, default_value = "cmd.done")]
        kind: String,

        /// Olay öznesi (pid, dosya yolu, pencere sınıfı)
        #[arg(long, default_value = "test-subject")]
        subject: String,

        /// Kaynak bağlamı (JSON formatında)
        #[arg(
            short,
            long,
            default_value = r#"{"exit_code": 101, "cmd": "cargo build"}"#
        )]
        ctx: String,

        /// Güven skoru (0.0 - 1.0)
        #[arg(long, default_value_t = 1.0)]
        conf: f64,
    },

    /// Canlı olay akışını dinle (Subscriber)
    Listen {
        /// Filtrelenecek olay türleri (örn: cmd.*, win.focus)
        #[arg(short, long)]
        kinds: Vec<String>,

        /// Filtrelenecek kaynaklar (örn: shell, hyprland)
        #[arg(short, long)]
        srcs: Vec<String>,
    },

    /// Graf sorgusu (RFC-001 §7.1: yapılandırılmış, salt-okunur)
    Query {
        /// Sorgu türü: "recent" (son olaylar) | "causal" (nedensellik kenarları)
        #[arg(short, long, default_value = "recent")]
        q: String,

        /// En fazla kaç kayıt
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    let mut client = BusClient::connect(cli.socket.as_deref()).await?;

    match cli.command {
        Commands::Ping => {
            let start = Instant::now();
            let req = WireRequest::Ping {
                req: "ping_1".to_string(),
            };
            let resp = client.call(&req).await?;
            let elapsed = start.elapsed();
            println!("🏓 Pong alındı! Gecikme: {:?}", elapsed);
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Commands::Status => {
            let req = WireRequest::Hello {
                req: "status_1".to_string(),
                client: "thornhill-ctl".to_string(),
                v: 1,
            };
            let resp = client.call(&req).await?;
            println!("👁️ Thornhill Daemon Durumu:");
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Commands::Inject {
            src,
            kind,
            subject,
            ctx,
            conf,
        } => {
            let ctx_val: serde_json::Value = serde_json::from_str(&ctx)
                .map_err(|e| format!("Geçersiz JSON ctx değeri: {}", e))?;

            let event = CanonicalEvent::new(src, kind, subject, ctx_val).with_confidence(conf);
            let event_id = event.id.clone();
            let event_ts = event.ts;

            let req = WireRequest::Event {
                req: "inject_1".to_string(),
                event,
            };

            let resp = client.call(&req).await?;
            println!("✅ Sentetik olay başarıyla enjekte edildi (grafe yazıldı)!");
            println!("   ID:  {}", event_id);
            println!("   TS:  {}", event_ts);
            println!("   Yanıt: {}", resp);
        }
        Commands::Listen { kinds, srcs } => {
            let filter = SubFilter { kinds, srcs };
            println!(
                "🎧 Thornhill Bus akışı dinleniyor (Ctrl+C ile çıkış)... Filtre: {:?}",
                filter
            );
            client.subscribe(filter).await?;

            while let Some(msg_res) = client.next_message().await {
                match msg_res {
                    Ok(WireResponse::EventNotification { op, event }) => {
                        println!(
                            "[{}] {} | src={} kind={} subject={} conf={:.2}",
                            op, event.id, event.src, event.kind, event.subject, event.conf
                        );
                        println!("     ctx: {}", event.ctx);
                    }
                    Ok(other) => {
                        println!("<<< {:?}", other);
                    }
                    Err(e) => {
                        eprintln!("Akış hatası: {}", e);
                        break;
                    }
                }
            }
        }
        Commands::Query { q, limit } => {
            let req = WireRequest::Query {
                req: "query_1".to_string(),
                q,
                limit,
            };
            let resp = client.call(&req).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
    }

    Ok(())
}
