//! Faz 6 — Thornhill Ambient Web Console (RFC-001 §7 ile aynı tel tipleri).
//!
//! Mimarî karar: elle yazılmış minimal HTTP/1.1 + SSE, doğrudan tokio üstünde.
//! Yeni bağımlılık YOK; UDS NDJSON protokolü (§7) hiç değişmedi — bu katman
//! yalnızca mevcut WireRequest tiplerini REST olarak dışa açar.
//!
//! Güvenlik sınırları:
//! - Varsayılan bağlama: `127.0.0.1:9099` (yalnız loopback).
//! - `THORNHILL_HTTP_ADDR` non-loopback ise, açık opt-in (`THORNHILL_HTTP_ALLOW_REMOTE=1`)
//!   olmadıkça sunucu BAŞLAMAZ.
//! - Mutasyon yollarında (Forget, ChipAct) `X-Thornhill-Token` başlığı zorunlu;
//!   token açılışta `$XDG_RUNTIME_DIR/thornhill/http.token` (0600) dosyasına yazılır.
//! - CSRF/DNS-rebinding: Host ve Origin başlıkları loopback olmayan isteklerde reddedilir.
//! - Statik dosyalarda path traversal koruması (sanitize_rel_path, birim testli).

use crate::actuator::{execute_action, ChipRegistry};
use crate::brain::Brain;
use crate::graph::GraphStore;
use crate::protocol::{CanonicalEvent, WireResponse};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Faz 6 HTTP sunucusu durumu — UDS bus ile AYNI Arc örneklerini paylaşır.
pub struct HttpState {
    pub graph: Arc<GraphStore>,
    pub brain: Arc<Brain>,
    pub registry: Arc<ChipRegistry>,
    pub event_tx: broadcast::Sender<CanonicalEvent>,
    pub chip_tx: broadcast::Sender<crate::protocol::ActionChip>,
    pub token: String,
    /// Statik dosyaların alternatif kaynağı (XDG override; ADR-062)
    pub dashboard_dir: Option<PathBuf>,
}

// ---------- Saf güvenlik fonksiyonları (birim testleri aşağıda) ----------

/// Host başlığı loopback mi? (porta izin verilir)
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            let ip = &host[1..end];
            return ip == "::1";
        }
    }
    let bare = host.split(':').next().unwrap_or(host);
    matches!(bare, "127.0.0.1" | "localhost" | "::1")
}

/// Origin başlığı CSRF/DNS-rebinding açısından izinli mi?
/// None (same-origin GET/fetch basit istek) veya loopback Origin kabul edilir.
pub fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(o) => {
            // Origin biçimi: scheme://host[:port]
            let host = o.split("://").nth(1).unwrap_or(o);
            is_loopback_host(host)
        }
    }
}

/// Statik dosya isteğinde traversal koruması: yalnız görece, ".."siz, gizli olmayan yol.
pub fn sanitize_rel_path(req: &str) -> Option<String> {
    if req.is_empty() || req == "/" {
        return Some("index.html".to_string());
    }
    if req.starts_with('/') {
        return None;
    }
    if req.contains("..") || req.contains('\\') || req.contains('\0') {
        return None;
    }
    // Gizli dosya/dizin engelle
    if req.split('/').any(|seg| seg.starts_with('.')) {
        return None;
    }
    Some(req.to_string())
}

pub fn token_ok(expected: &str, got: Option<&str>) -> bool {
    match got {
        Some(g) => {
            if g.len() != expected.len() {
                return false;
            }
            g.bytes()
                .zip(expected.bytes())
                .map(|(a, b)| a ^ b)
                .fold(0u8, |acc, x| acc | x)
                == 0
        }
        None => false,
    }
}

/// Açılış token'ı: /dev/urandom'dan 16 bayt hex. Soket dizinine 0600 ile yazılır.
pub fn create_token_file() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    let dir = PathBuf::from(runtime).join("thornhill");
    std::fs::create_dir_all(&dir)?;

    let mut bytes = [0u8; 16];
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    }
    let token: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();

    let path = dir.join("http.token");
    std::fs::write(&path, &token)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(token)
}

// ---------- HTTP sunucusu ----------

pub struct HttpServer;

impl HttpServer {
    /// Bağlama adresini çözer; non-loopback açık opt-in olmadan reddedilir.
    pub fn resolve_addr() -> Result<String, String> {
        let addr =
            std::env::var("THORNHILL_HTTP_ADDR").unwrap_or_else(|_| "127.0.0.1:9099".to_string());
        let host = addr.split(':').next().unwrap_or(&addr).to_string();
        if !is_loopback_host(&host) {
            let optin = std::env::var("THORNHILL_HTTP_ALLOW_REMOTE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if !optin {
                return Err(format!(
                    "THORNHILL_HTTP_ADDR '{}' loopback değil. Ağ erişimi için \
                     THORNHILL_HTTP_ALLOW_REMOTE=1 gereklidir (RFC-001 §8.1).",
                    addr
                ));
            }
            warn!("⚠️ HTTP konsolu AĞA AÇIK: {} (opt-in onaylandı)", addr);
        }
        Ok(addr)
    }

    pub async fn run(
        state: Arc<HttpState>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = match Self::resolve_addr() {
            Ok(a) => a,
            Err(e) => {
                error!("{}", e);
                return Err(e.into());
            }
        };
        let listener = TcpListener::bind(&addr).await?;
        info!(
            "🌐 Web konsolu: http://{} (token: $XDG_RUNTIME_DIR/thornhill/http.token)",
            addr
        );

        loop {
            let (stream, _peer) = listener.accept().await?;
            // UDS'teki SO_PEERCRED'in HTTP karşılığı: bağlantı zaten TCP; koruma
            // loopback bağlaması + Host/Origin + token üçlüsüdür (yukarıda).
            let st = Arc::clone(&state);
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, st).await {
                    debug_drop(e);
                }
            });
        }
    }
}

fn debug_drop<E: std::fmt::Display>(e: E) {
    tracing::debug!("HTTP bağlantısı kapandı: {}", e);
}

async fn handle_conn(
    stream: TcpStream,
    state: Arc<HttpState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    // İstek satırı + başlıklar
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut headers: HashMap<String, String> = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }

    // Host doğrulaması (DNS rebinding karşıtı) — tüm istekler
    let host_ok = headers
        .get("host")
        .map(|h| is_loopback_host(h))
        .unwrap_or(false);
    if !host_ok {
        return respond(&mut writer, 403, "text/plain", b"forbidden host\n").await;
    }

    // Body oku (mutasyonlar küçük JSON; 64 KB üstü reddedilir)
    let content_len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if content_len > 64 * 1024 {
        return respond(&mut writer, 413, "text/plain", b"payload too large\n").await;
    }
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        reader.read_exact(&mut body).await?;
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.clone(), None),
    };

    // SSE: canlı olay + çip akışı (GET)
    if path == "/api/stream" && method == "GET" {
        return stream_sse(writer, state).await;
    }

    match (method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            serve_static(&mut writer, &state, "index.html").await
        }
        ("GET", "/style.css") => serve_static(&mut writer, &state, "style.css").await,
        ("GET", "/app.js") => serve_static(&mut writer, &state, "app.js").await,
        ("GET", "/d3.v7.min.js") => serve_static(&mut writer, &state, "d3.v7.min.js").await,

        // Dashboard mutasyonları için token (yalnız loopback + Host/Origin koruması
        // altında sunulur; CORS başlığı hiçbir zaman eklenmez — çapraz köken JS okuyamaz)
        ("GET", "/api/token") => {
            respond_json(
                &mut writer,
                200,
                &serde_json::json!({ "token": state.token }),
            )
            .await
        }

        ("GET", "/api/stats") => match state.graph.stats() {
            Ok(s) => respond_json(&mut writer, 200, &s).await,
            Err(e) => {
                respond_json(
                    &mut writer,
                    500,
                    &serde_json::json!({"error": e.to_string()}),
                )
                .await
            }
        },
        ("GET", "/api/query") => {
            let params = parse_query(query.as_deref());
            let q = params
                .get("q")
                .cloned()
                .unwrap_or_else(|| "recent".to_string());
            let limit = params
                .get("limit")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(50);
            match state.graph.query(&q, limit) {
                Ok(rows) => {
                    respond_json(&mut writer, 200, &serde_json::json!({"rows": rows})).await
                }
                Err(e) => {
                    respond_json(
                        &mut writer,
                        400,
                        &serde_json::json!({"error": e.to_string()}),
                    )
                    .await
                }
            }
        }

        ("POST", "/api/ask") | ("POST", "/api/forget") | ("POST", "/api/chip-act") => {
            // CSRF/DNS-rebinding: Origin ya yok ya loopback olmalı
            if !origin_allowed(headers.get("origin").map(|s| s.as_str())) {
                return respond(&mut writer, 403, "text/plain", b"forbidden origin\n").await;
            }
            // Mutasyon koruması: token başlığı (Forget, ChipAct; Ask için de önerilir)
            if path == "/api/forget" || path == "/api/chip-act" {
                if !token_ok(
                    &state.token,
                    headers.get("x-thornhill-token").map(|s| s.as_str()),
                ) {
                    return respond(
                        &mut writer,
                        401,
                        "text/plain",
                        b"missing or invalid token\n",
                    )
                    .await;
                }
            }
            let v: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(e) => {
                    return respond_json(
                        &mut writer,
                        400,
                        &serde_json::json!({"error": format!("bad json: {}", e)}),
                    )
                    .await
                }
            };

            match path.as_str() {
                "/api/ask" => {
                    let question = v
                        .get("question")
                        .and_then(|q| q.as_str())
                        .unwrap_or("")
                        .to_string();
                    if question.is_empty() {
                        return respond_json(
                            &mut writer,
                            400,
                            &serde_json::json!({"error": "question required"}),
                        )
                        .await;
                    }
                    match state.brain.answer_query(&question).await {
                        Ok(ans) => {
                            let _ = state.graph.append(&CanonicalEvent::new(
                                "user",
                                "s2_query",
                                &question,
                                serde_json::json!({"confidence": ans.confidence, "via": "http"}),
                            ));
                            respond_json(
                                &mut writer,
                                200,
                                &serde_json::json!({
                                    "explanation": ans.explanation,
                                    "remedy": ans.remedy,
                                    "confidence": ans.confidence,
                                }),
                            )
                            .await
                        }
                        Err(e) => {
                            respond_json(&mut writer, 502, &serde_json::json!({"error": e})).await
                        }
                    }
                }
                "/api/forget" => {
                    let before_ts = v.get("before_ts").and_then(|b| b.as_i64()).unwrap_or(0);
                    match state.graph.forget_before(before_ts) {
                        Ok((ne, nd)) => {
                            respond_json(
                                &mut writer,
                                200,
                                &serde_json::json!({
                                    "forgotten_events": ne, "forgotten_edges": nd
                                }),
                            )
                            .await
                        }
                        Err(e) => {
                            respond_json(
                                &mut writer,
                                500,
                                &serde_json::json!({"error": e.to_string()}),
                            )
                            .await
                        }
                    }
                }
                "/api/chip-act" => {
                    let chip_id = v
                        .get("chip_id")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    let confirm = v.get("confirm").and_then(|c| c.as_bool()).unwrap_or(false);
                    let action_label = v
                        .get("action_label")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string());

                    let chip =
                        match state.registry.peek(&chip_id) {
                            Some(c) => c,
                            None => {
                                let _ = state.graph.append(&CanonicalEvent::new(
                                "user", "chip.dismiss", &chip_id,
                                serde_json::json!({"reason": "expired_or_unknown", "via": "http"}),
                            ));
                                return respond_json(
                                    &mut writer,
                                    404,
                                    &serde_json::json!({"error": "chip not found or expired"}),
                                )
                                .await;
                            }
                        };
                    let action = match action_label
                        .as_ref()
                        .and_then(|label| chip.actions.iter().find(|a| &a.label == label))
                        .or_else(|| chip.actions.first())
                    {
                        Some(a) => a.clone(),
                        None => {
                            return respond_json(
                                &mut writer,
                                400,
                                &serde_json::json!({"error": "no actionable element"}),
                            )
                            .await
                        }
                    };

                    if !confirm {
                        use crate::actuator::ladder_check;
                        return match ladder_check(&action.risk, false) {
                            crate::actuator::Decision::Refused(reason) => {
                                respond_json(
                                    &mut writer,
                                    403,
                                    &serde_json::json!({"error": reason}),
                                )
                                .await
                            }
                            _ => {
                                respond_json(
                                    &mut writer,
                                    200,
                                    &serde_json::json!({
                                        "dispatched": false,
                                        "requires": "confirm=true",
                                        "action": action.action_type,
                                        "risk": action.risk,
                                    }),
                                )
                                .await
                            }
                        };
                    }

                    if action.action_type == "ask_llm" {
                        let question = action
                            .payload
                            .get("question")
                            .and_then(|q| q.as_str())
                            .unwrap_or("Bu hatayı açıkla")
                            .to_string();
                        return match state.brain.answer_chip(&question).await {
                            Ok(ans) => {
                                let _ = state.graph.append(&CanonicalEvent::new(
                                    "user",
                                    "chip.accept",
                                    &chip_id,
                                    serde_json::json!({"action": "ask_llm", "via": "http"}),
                                ));
                                respond_json(&mut writer, 200, &serde_json::json!({
                                    "dispatched": true, "explanation": ans.explanation, "remedy": ans.remedy,
                                })).await
                            }
                            Err(e) => {
                                respond_json(&mut writer, 502, &serde_json::json!({"error": e}))
                                    .await
                            }
                        };
                    }

                    let outcome = execute_action(&action, &chip).await;
                    let _ = state.graph.append(&CanonicalEvent::new(
                        "user",
                        "chip.accept",
                        &chip_id,
                        serde_json::json!({"action": action.action_type, "via": "http"}),
                    ));
                    state.registry.remove(&chip_id);
                    let (code, payload) = match outcome {
                        crate::actuator::Outcome::Exec {
                            exit_code,
                            stdout,
                            stderr,
                        } => (
                            200,
                            serde_json::json!({"dispatched": true, "exit_code": exit_code, "stdout": stdout, "stderr": stderr}),
                        ),
                        crate::actuator::Outcome::DryRunOk { note } => (
                            200,
                            serde_json::json!({"dispatched": true, "dry_run": true, "note": note}),
                        ),
                        crate::actuator::Outcome::Refused { reason } => {
                            (403, serde_json::json!({"error": reason}))
                        }
                    };
                    respond_json(&mut writer, code, &payload).await
                }
                _ => unreachable!(),
            }
        }

        _ => respond(&mut writer, 404, "text/plain", b"not found\n").await,
    }
}

/// Minimal sorgu-dizesi ayrıştırıcı (harici url/form crate bağımlılığı YOK).
fn parse_query(q: Option<&str>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(q) = q {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                out.insert(
                    k.replace("%20", " ").replace('+', " "),
                    v.replace("%20", " ").replace('+', " "),
                );
            }
        }
    }
    out
}

/// SSE akışı: event.push ve chip.push bildirimlerini tarayıcıya canlı basar.
/// RFC-001 §7'deki broadcast kanallarını doğrudan kullanır — protokol değişmez.
async fn stream_sse(
    mut writer: BufWriter<tokio::net::tcp::OwnedWriteHalf>,
    state: Arc<HttpState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Başlık
    writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n").await?;
    writer.flush().await?;

    let mut event_rx = state.event_tx.subscribe();
    let mut chip_rx = state.chip_tx.subscribe();

    // İstemci kopması yazma hatasıyla tespit edilir (send başarısız → break);
    // okuma yarısını seçime sokmak borrowck'a gereksiz yük olur.
    loop {
        tokio::select! {
            res = event_rx.recv() => {
                match res {
                    Ok(ev) => {
                        let payload = WireResponse::event_push(ev);
                        let line = serde_json::to_string(&payload)?;
                        writer.write_all(format!("data: {}\n\n", line).as_bytes()).await?;
                        writer.flush().await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let line = serde_json::to_string(&serde_json::json!({"op":"stream.lagged","missed":n}))?;
                        writer.write_all(format!("data: {}\n\n", line).as_bytes()).await?;
                        writer.flush().await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            res = chip_rx.recv() => {
                match res {
                    Ok(chip) => {
                        let payload = WireResponse::chip_push(chip);
                        let line = serde_json::to_string(&payload)?;
                        writer.write_all(format!("data: {}\n\n", line).as_bytes()).await?;
                        writer.flush().await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let line = serde_json::to_string(&serde_json::json!({"op":"stream.lagged","missed":n}))?;
                        writer.write_all(format!("data: {}\n\n", line).as_bytes()).await?;
                        writer.flush().await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    Ok(())
}

async fn serve_static(
    writer: &mut BufWriter<tokio::net::tcp::OwnedWriteHalf>,
    state: &HttpState,
    rel: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Path traversal koruması (birim testli)
    let safe = match sanitize_rel_path(rel) {
        Some(s) => s,
        None => return respond(writer, 403, "text/plain", b"forbidden path\n").await,
    };

    // 1) XDG override dizini (ADR-062), 2) binary'ye gömülü (include_str!)
    let override_path = state.dashboard_dir.as_ref().map(|d| d.join(&safe));
    let bytes: Option<Vec<u8>> = match override_path {
        Some(p) if p.exists() => std::fs::read(&p).ok(),
        _ => None,
    };
    let bytes = match bytes {
        Some(b) => Some(b),
        None => match safe.as_str() {
            "index.html" => Some(include_bytes!("../../dashboard/index.html").to_vec()),
            "style.css" => Some(include_bytes!("../../dashboard/style.css").to_vec()),
            "app.js" => Some(include_bytes!("../../dashboard/app.js").to_vec()),
            "d3.v7.min.js" => Some(include_bytes!("../../dashboard/d3.v7.min.js").to_vec()),
            _ => None,
        },
    };

    match bytes {
        Some(b) => {
            let mime = match safe.rsplit('.').next() {
                Some("html") => "text/html; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("js") => "application/javascript; charset=utf-8",
                _ => "application/octet-stream",
            };
            respond(writer, 200, mime, &b).await
        }
        None => respond(writer, 404, "text/plain", b"not found\n").await,
    }
}

async fn respond(
    writer: &mut BufWriter<tokio::net::tcp::OwnedWriteHalf>,
    code: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let reason = match code {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        _ => "Error",
    };
    writer
        .write_all(
            format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                code, reason, content_type, body.len()
            )
            .as_bytes(),
        )
        .await?;
    writer.write_all(body).await?;
    writer.flush().await?;
    Ok(())
}

async fn respond_json(
    writer: &mut BufWriter<tokio::net::tcp::OwnedWriteHalf>,
    code: u16,
    v: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    respond(
        writer,
        code,
        "application/json; charset=utf-8",
        v.to_string().as_bytes(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_host_detection() {
        assert!(is_loopback_host("127.0.0.1:9099"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("[::1]:8080"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("evil.example.com"));
        assert!(!is_loopback_host("192.168.1.10"));
    }

    #[test]
    fn origin_check_blocks_rebinding() {
        assert!(origin_allowed(None)); // same-origin basit istek
        assert!(origin_allowed(Some("http://127.0.0.1:9099")));
        assert!(origin_allowed(Some("http://localhost:9099")));
        assert!(!origin_allowed(Some("http://evil.example.com")));
        assert!(!origin_allowed(Some("http://127.0.0.1.evil.com")));
    }

    #[test]
    fn path_traversal_blocked() {
        assert_eq!(sanitize_rel_path(""), Some("index.html".into()));
        assert_eq!(sanitize_rel_path("/"), Some("index.html".into()));
        assert_eq!(sanitize_rel_path("app.js"), Some("app.js".into()));
        assert_eq!(sanitize_rel_path("index.html"), Some("index.html".into()));
        assert_eq!(sanitize_rel_path("../etc/passwd"), None);
        assert_eq!(sanitize_rel_path("a/../../b"), None);
        assert_eq!(sanitize_rel_path("/etc/passwd"), None);
        assert_eq!(sanitize_rel_path("a\\b"), None);
        assert_eq!(sanitize_rel_path(".hidden"), None);
    }

    #[test]
    fn token_check_rejects_missing_or_wrong() {
        assert!(!token_ok("abc", None));
        assert!(!token_ok("abc", Some("xyz")));
        assert!(!token_ok("abc", Some("abcd"))); // uzunluk eşitsizliği
        assert!(token_ok("abc", Some("abc")));
    }
}
