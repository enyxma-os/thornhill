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
    /// Sentetik kanonik olay enjekte et (S1'i tetikler)
    Inject {
        #[arg(short, long, default_value = "shell")]
        src: String,
        #[arg(short, long, default_value = "cmd.done")]
        kind: String,
        #[arg(long, default_value = "test-subject")]
        subject: String,
        #[arg(
            short,
            long,
            default_value = r#"{"exit_code": 101, "cmd": "cargo build"}"#
        )]
        ctx: String,
        #[arg(long, default_value_t = 1.0)]
        conf: f64,
    },
    /// Canlı olay + çip akışını dinle (Subscriber)
    Listen {
        #[arg(short, long)]
        kinds: Vec<String>,
        #[arg(short, long)]
        srcs: Vec<String>,
        /// S1 çiplerini de dinle (chip.push)
        #[arg(long, default_value_t = true)]
        chips: bool,
    },
    /// Graf sorgusu (RFC-001 §7.1: yapılandırılmış, salt-okunur)
    Query {
        #[arg(short, long, default_value = "recent")]
        q: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Action Chip eylemini onayla veya önizle (§8.5 risk merdiveni)
    ChipAct {
        /// Çip kimliği (örn: chip_01M5...)
        chip_id: String,

        /// Eylem etiketi (boşsa ilk eylem seçilir)
        #[arg(short, long)]
        action: Option<String>,

        /// Eylemi kesin olarak onayla (hold-to-confirm)
        #[arg(long, default_value_t = false)]
        confirm: bool,
    },
    /// S2 bilişsel katmana soru sor (RFC-001 §9.3.3: her zaman S2)
    Ask {
        /// Soru metni (örn: "son derleme hatası neden kaynaklandı?")
        question: String,
    },
    /// Unutma hakkı (RFC-001 §8.6): eski olayları grafte sil
    Forget {
        /// Bundan eskisini sil; ör. "1d", "7d", "30d" veya epoch ms
        #[arg(long, default_value = "30d")]
        before: String,
    },
}

/// "1d"/"7d"/"30d" gibi göreceli süreleri epoch ms'e çevirir.
fn parse_before(s: &str) -> Result<i64, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    if let Ok(ms) = s.parse::<i64>() {
        return Ok(now - ms);
    }

    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num.parse().map_err(|_| format!("Geçersiz süre: {}", s))?;
    let days = match unit {
        "d" => n,
        "h" => return Ok(now - n * 3_600_000),
        _ => {
            return Err(format!(
                "Geçersiz süre birimi: {} (d veya h kullanın)",
                unit
            ))
        }
    };
    Ok(now - days * 86_400_000)
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

            let req = WireRequest::Event {
                req: "inject_1".to_string(),
                event,
            };
            let resp = client.call(&req).await?;
            println!("✅ Olay enjekte edildi (graf + S1 değerlendirmesi)!");
            println!("   ID:  {}", event_id);
            println!("   Yanıt: {}", resp);
        }
        Commands::Listen { kinds, srcs, chips } => {
            let filter = SubFilter { kinds, srcs, chips };
            println!(
                "🎧 Thornhill akışı dinleniyor (Ctrl+C ile çıkış)... chips={}. Filtre: {:?}",
                if chips { "açık" } else { "kapalı" },
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
                    Ok(WireResponse::ChipPush { op, chip }) => {
                        if chips {
                            println!("\n⚡ [{}] {} | {}", op, chip.id, chip.title);
                            println!("     why:     {}", chip.why);
                            println!("     trigger: {}", chip.trigger);
                            println!("     çipler:  {}", chip.labels.join(" | "));
                            println!("     ttl:     {} ms, conf: {:.2}", chip.ttl_ms, chip.conf);
                        }
                    }
                    Ok(other) => println!("<<< {:?}", other),
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
        Commands::ChipAct {
            chip_id,
            action,
            confirm,
        } => {
            let req = WireRequest::ChipAct {
                req: "act_1".to_string(),
                chip_id,
                action_label: action,
                confirm,
            };
            let resp = client.call(&req).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Commands::Ask { question } => {
            println!("🧠 S2 düşünüyor: {:?}", question);
            let start = Instant::now();
            let req = WireRequest::Ask {
                req: "ask_1".to_string(),
                question: question.clone(),
            };
            let resp = client.call(&req).await?;
            println!("(süre: {:?})", start.elapsed());
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Commands::Forget { before } => {
            let before_ts = parse_before(&before)?;
            let req = WireRequest::Forget {
                req: "forget_1".to_string(),
                before_ts,
            };
            let resp = client.call(&req).await?;
            println!("🧹 Unutma işlemi tamamlandı (before={})", before);
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
    }

    Ok(())
}
