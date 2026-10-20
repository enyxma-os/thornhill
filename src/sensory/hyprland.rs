//! RFC-001 §4.1 — Hyprland socket2 duyusu.
//!
//! `$XDG_RUNTIME_DIR/hypr/<HIS>/.socket2.sock` hattına bağlanır, ham olay
//! satırlarını kanonik olay şemasına (§5) çevirir ve bus'a aktarır.
//!
//! Tasarım notları:
//! - **Event-driven:** Tek okuma döngüsü, soketten satır gelir; poll yok
//!   (Aksiyom 2). Yeniden bağlanma üstel geri çekilmeli yönetim mantığıdır,
//!   periyodik tarama değildir.
//! - **Keşif:** Önce `HYPRLAND_INSTANCE_SIGNATURE` env'i; yoksa `hypr/` altında
//!   socket2.sock içeren tek dizin (ADR-062: sabit kullanıcı yolu yasak).
//! - **Duyu asla daemon'ı bloklamaz:** Bağlantı kopukluğu yalnızca uyarı
//!   üretir; bus, graf ve UDS istemcileri çalışmaya devam eder.

use crate::protocol::CanonicalEvent;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::unix::OwnedReadHalf;
use tokio::net::UnixStream;
use tracing::{debug, info, warn};

/// Olay türlerini sınıflandıran geri çağrı: duyu katmanı bus'a bağımlı olmasın.
pub type EventSink = tokio::sync::mpsc::UnboundedSender<CanonicalEvent>;

/// socket2 yolunu çözer (ADR-062 uyumlu keşif).
pub fn discover_socket_path() -> Option<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;

    // 1) Açık imza varsa doğrudan
    if let Ok(his) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        let p = PathBuf::from(&runtime)
            .join("hypr")
            .join(&his)
            .join(".socket2.sock");
        if p.exists() {
            return Some(p);
        }
    }

    // 2) Fallback: hypr/ altında .socket2.sock içeren tek dizin
    let hypr_dir = PathBuf::from(&runtime).join("hypr");
    let entries = std::fs::read_dir(&hypr_dir).ok()?;
    let mut found: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let candidate = entry.path().join(".socket2.sock");
        if candidate.exists() {
            if found.is_some() {
                // Birden fazla oturum belirsizliği: açık imza olmadan karar veremeyiz.
                warn!("hypr/ altında birden fazla socket2 oturumu bulundu; HYPRLAND_INSTANCE_SIGNATURE gerekli");
                return None;
            }
            found = Some(candidate);
        }
    }
    found
}

/// socket2 olay satırını (RFC: `EVENT_NAME>>DATA`) kanonik olaya çevirir.
/// Genel yardımcı fonksiyon — birim testleri burada.
pub fn parse_socket2_line(line: &str, ts: u64) -> Option<CanonicalEvent> {
    let (name, data) = line.split_once(">>")?;
    let mut ev = match name {
        // workspace>>WSNAME — örn. "2:code" veya "3"
        "workspace" => CanonicalEvent::new(
            "hyprland",
            "ws.focus",
            data,
            serde_json::json!({ "ws": data }),
        ),
        // focusedmon>>MON,WS
        "focusedmon" => {
            let (mon, ws) = data.split_once(',')?;
            CanonicalEvent::new(
                "hyprland",
                "ws.focus",
                format!("monitor:{}", mon),
                serde_json::json!({ "monitor": mon, "ws": ws }),
            )
        }
        // openwindow>>ADDR,WORKSPACE,CLASS,TITLE
        "openwindow" => {
            let mut f = data.splitn(4, ',');
            let addr = f.next()?;
            let ws = f.next().unwrap_or("");
            let class = f.next().unwrap_or("");
            let title = f.next().unwrap_or("");
            CanonicalEvent::new(
                "hyprland",
                "win.open",
                format!("{}:{}", class, addr),
                serde_json::json!({ "ws": ws, "wm_class": class, "title": title }),
            )
        }
        // closewindow>>ADDR
        "closewindow" => CanonicalEvent::new(
            "hyprland",
            "win.close",
            data,
            serde_json::json!({ "addr": data }),
        ),
        // activewindow>>CLASS,TITLE
        "activewindow" => {
            let mut f = data.splitn(2, ',');
            let class = f.next()?;
            let title = f.next().unwrap_or("");
            // Boş class = odak boşaldı (pencere kapandı); olay olarak yine değerli.
            CanonicalEvent::new(
                "hyprland",
                "win.focus",
                class,
                serde_json::json!({ "wm_class": class, "title": title }),
            )
        }
        // activewindowv2>>ADDR (yalnız adres; class/title için zaten activewindow gelir)
        "activewindowv2" => {
            return None; // coalesce: activewindow zaten tam bağlamı taşır
        }
        // fullscreen>>0|1
        "fullscreen" => CanonicalEvent::new(
            "hyprland",
            "win.fullscreen",
            "active",
            serde_json::json!({ "state": data }),
        ),
        // monitoradded>>NAME
        "monitoradded" => CanonicalEvent::new(
            "hyprland",
            "monitor.add",
            data,
            serde_json::json!({ "monitor": data }),
        ),
        _ => {
            debug!("Bilinmeyen socket2 olayı atlandı: {}", name);
            return None;
        }
    };
    ev.ts = ts; // alan pub: RFC-001 §5 zaman damgası duyuda damgalanır
    Some(ev)
}

impl HyprlandSensory {
    /// Bir socket2 satır dizisini işleyip kanonik olaylara çevirir (test için).
    pub fn parse_lines(lines: &[&str], ts: u64) -> Vec<CanonicalEvent> {
        lines
            .iter()
            .filter_map(|l| parse_socket2_line(l, ts))
            .collect()
    }
}

pub struct HyprlandSensory;

impl HyprlandSensory {
    /// Duyu döngüsü: bağlan, satır satır oku, kanonik olayları sink'e yaz.
    /// Bağlantı koparsa üstel geri çekilmeyle (1s→30s üst sınır) yeniden dener.
    pub async fn run(sink: EventSink) {
        let mut backoff_secs: u64 = 1;

        loop {
            let path = match discover_socket_path() {
                Some(p) => p,
                None => {
                    // Hyprland yok/belirsiz: uzun bekleme, tekrar dene.
                    // Bu bir poll değil; oturum yaşam döngüsü yönetimi.
                    warn!(
                        "Hyprland socket2 bulunamadı; {} sn sonra tekrar deneniyor",
                        backoff_secs
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(30);
                    continue;
                }
            };

            info!("👁️ Hyprland socket2 hattına bağlanılıyor: {:?}", path);
            match UnixStream::connect(&path).await {
                Ok(stream) => {
                    backoff_secs = 1; // başarılı bağlantı, geri çekilmeyi sıfırla
                    info!("Hyprland duyu hattı kuruldu: {:?}", path);

                    let (read_half, _write) = stream.into_split();
                    let mut lines = BufReader::new(read_half);

                    loop {
                        match Self::read_line(&mut lines).await {
                            Ok(Some(line)) => {
                                let ts = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0);
                                if let Some(ev) = parse_socket2_line(&line, ts) {
                                    debug!("hyprland olayı: {} {}", ev.kind, ev.subject);
                                    if sink.send(ev).is_err() {
                                        info!("Olay havuzu kapandı; duyu döngüsü sonlanıyor");
                                        return;
                                    }
                                }
                            }
                            Ok(None) => {
                                warn!("Hyprland socket2 kapandı (EOF); yeniden bağlanılıyor");
                                break;
                            }
                            Err(e) => {
                                warn!("socket2 okuma hatası: {}; yeniden bağlanılıyor", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "socket2'ye bağlanılamadı ({}); {} sn sonra tekrar denenecek",
                        e, backoff_secs
                    );
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(30);
        }
    }

    /// OwnedReadHalf üzerinde satır okuma (AsyncBufReadExt, buffer'ı korur).
    async fn read_line(reader: &mut BufReader<OwnedReadHalf>) -> std::io::Result<Option<String>> {
        let mut buf = String::new();
        match reader.read_line(&mut buf).await {
            Ok(0) => Ok(None), // EOF
            Ok(_) => Ok(Some(buf.trim_end_matches(['\n', '\r']).to_string())),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workspace_focus() {
        let ev = parse_socket2_line("workspace>>2:code", 1000).unwrap();
        assert_eq!(ev.src, "hyprland");
        assert_eq!(ev.kind, "ws.focus");
        assert_eq!(ev.subject, "2:code");
        assert_eq!(ev.ctx["ws"], "2:code");
        assert_eq!(ev.ts, 1000);
        assert_eq!(ev.conf, 1.0);
    }

    #[test]
    fn parses_openwindow_full_context() {
        let ev = parse_socket2_line("openwindow>>cafe1,2,kitty,nvim rfc-001.md", 2000).unwrap();
        assert_eq!(ev.kind, "win.open");
        assert_eq!(ev.subject, "kitty:cafe1");
        assert_eq!(ev.ctx["wm_class"], "kitty");
        assert_eq!(ev.ctx["title"], "nvim rfc-001.md");
        assert_eq!(ev.ctx["ws"], "2");
    }

    #[test]
    fn parses_active_window_focus() {
        let ev = parse_socket2_line("activewindow>>firefox,Documentation — Rust", 3000).unwrap();
        assert_eq!(ev.kind, "win.focus");
        assert_eq!(ev.subject, "firefox");
        assert_eq!(ev.ctx["title"], "Documentation — Rust");
    }

    #[test]
    fn skips_v2_and_unknown() {
        assert!(parse_socket2_line("activewindowv2>>cafe1", 0).is_none());
        assert!(parse_socket2_line("unknown>>x", 0).is_none());
        assert!(parse_socket2_line("", 0).is_none());
    }

    #[test]
    fn parses_focusedmon() {
        let ev = parse_socket2_line("focusedmon>>DP-1,2", 4000).unwrap();
        assert_eq!(ev.kind, "ws.focus");
        assert_eq!(ev.subject, "monitor:DP-1");
        assert_eq!(ev.ctx["monitor"], "DP-1");
        assert_eq!(ev.ctx["ws"], "2");
    }

    #[test]
    fn batch_parses_lines() {
        let lines = vec![
            "workspace>>1",
            "openwindow>>a1,1,kitty,term",
            "activewindowv2>>a1",
        ];
        let events = HyprlandSensory::parse_lines(&lines, 5000);
        assert_eq!(events.len(), 2); // v2 coalesce edildi
    }
}
