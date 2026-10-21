use crate::actuator::{execute_action, ChipRegistry};
use crate::brain::Brain;
use crate::graph::GraphStore;
use crate::protocol::{ActionChip, CanonicalEvent, SubFilter, WireRequest, WireResponse};
use crate::reflex::{is_escalation_candidate, ReflexEngine};
use futures_util::{SinkExt, StreamExt};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};
use tracing::{debug, error, info, warn};

/// UDS Soket yolunu çözümler ($XDG_RUNTIME_DIR/thornhill/thornhill.sock)
pub fn resolve_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir)
            .join("thornhill")
            .join("thornhill.sock")
    } else {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{}/thornhill/thornhill.sock", uid))
    }
}

/// §6.3 korelasyon penceresi (ms)
const CORRELATION_WINDOW_MS: u64 = 10_000;

/// Faz 5: S1→S2 otomatik yükseltme anahtarı (varsayılan kapalı — yalnız günlük).
fn auto_escalate_enabled() -> bool {
    std::env::var("THORNHILL_S2_AUTO")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub struct BusServer {
    socket_path: PathBuf,
    event_tx: broadcast::Sender<CanonicalEvent>,
    chip_tx: broadcast::Sender<ActionChip>,
    graph: Arc<GraphStore>,
    reflex: Arc<ReflexEngine>,
    registry: Arc<ChipRegistry>,
    brain: Arc<Brain>,
}

impl BusServer {
    pub fn new(socket_path: Option<PathBuf>, graph: Arc<GraphStore>, brain: Arc<Brain>) -> Self {
        let path = socket_path.unwrap_or_else(resolve_socket_path);
        let (event_tx, _) = broadcast::channel(1024);
        let (chip_tx, _) = broadcast::channel(256);

        Self {
            socket_path: path,
            event_tx,
            chip_tx,
            graph,
            reflex: Arc::new(ReflexEngine::new()),
            registry: Arc::new(ChipRegistry::new()),
            brain,
        }
    }

    /// Duyu modülleri için içsel besleme yolu.
    /// Faz 5: G3 ölçümü artık TAM boru hattını kapsar (denetim bulgu #6).
    pub fn ingest(&self, event: CanonicalEvent) -> bool {
        let pipeline_start = std::time::Instant::now();

        // 1) Kalıcılaştır (edges foreign key şartı)
        if let Err(e) = self.graph.append(&event) {
            error!("Duyu olayı grafe yazılamadı: {}", e);
            return false;
        }
        let _ = self.event_tx.send(event.clone());

        let mut s1_fired = false;

        // 2) §6.3 korelasyon
        if event.src == "shell" && event.kind == "cmd.done" {
            let failed = event
                .ctx
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .map(|c| c != 0)
                .unwrap_or(false);
            if failed {
                if let Ok(prior) = self.graph.recent_within(CORRELATION_WINDOW_MS) {
                    for (pid, kind, _subject) in &prior {
                        if pid == &event.id {
                            continue;
                        }
                        let relevant = matches!(
                            kind.as_str(),
                            "git.dirty" | "git.commit" | "fs.write" | "proc.exec"
                        );
                        if relevant {
                            if let Ok(support) = self.graph.link(pid, &event.id, "caused", 0.5) {
                                debug!(
                                    "§6.3 kenar: {} → {} (support={})",
                                    kind, event.kind, support
                                );
                                if support >= crate::graph::db::LESSON_SUPPORT_THRESHOLD {
                                    let pattern = format!("{}->{}", kind, event.kind);
                                    let summary = format!(
                                        "{} olayı {} içinde {} kez {} ile sonuçlandı",
                                        kind, CORRELATION_WINDOW_MS, support, event.kind
                                    );
                                    if let Ok(true) = self.graph.ensure_lesson(
                                        &pattern,
                                        &summary,
                                        Some("komutu düzelt veya diff uygula"),
                                    ) {
                                        info!("📚 Yeni lesson öğrenildi: {}", pattern);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3) S1 refleks (§9.1)
        if let Some(chip) = self.reflex.evaluate(&event) {
            s1_fired = true;
            debug!("S1 çip üretti: {} ({})", chip.id, chip.title);
            self.registry.insert(chip.clone());
            let _ = self.chip_tx.send(chip);
        } else if is_escalation_candidate(&event, false) {
            // Faz 5 — §9.3 karar günlüğü: S1'in kaçırdığı sistemik hata kayda geçer.
            let auto = auto_escalate_enabled();
            let _ = self.graph.append(&CanonicalEvent::new(
                "user",
                "s1_escalate",
                &event.id,
                serde_json::json!({
                    "criteria": "9.3.1-yeni-desen",
                    "auto": auto
                }),
            ));
            // Otomatik yükseltme yalnız THORNHILL_S2_AUTO=1 ile (maliyet koruması §10);
            // varsayılan davranış: yalnız kaydet, kullanıcı sorarsa S2 (§9.3.3).
            if auto {
                // Bilinçli olarak SESSİZ: otomatik S2 çağrısı rate limit'e tabidir;
                // sonuç chip kanalından döner (Faz 6'da zenginleştirilecek).
                let brain = Arc::clone(&self.brain);
                let question = format!(
                    "Sistemde açıklanamayan bir hata oluştu: {} {} — neden olabilir?",
                    event.kind,
                    event
                        .ctx
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("")
                );
                tokio::spawn(async move {
                    if let Err(e) = brain.answer_chip(&question).await {
                        debug!("Otomatik S2 yükseltmesi atlandı: {}", e);
                    }
                });
            }
        }

        // Faz 5 — G3: TAM boru hattı ölçümü (kalıcılık + korelasyon + S1 + günlük)
        let elapsed = pipeline_start.elapsed();
        if elapsed > std::time::Duration::from_millis(1) {
            tracing::warn!(
                "⚠️ G3 boru hattı ihlali: ingest toplam {:?} (bütçe <1 ms; yalnız S1 kuralı değil)",
                elapsed
            );
        } else {
            debug!(
                "ingest boru hattı: {:?} (bütçe içinde, s1_fired={})",
                elapsed, s1_fired
            );
        }
        true
    }

    // ---- Faz 6: HTTP katmanıyla paylaşım (UDS protokolü değişmeden) ----
    pub fn graph(&self) -> Arc<GraphStore> {
        Arc::clone(&self.graph)
    }
    pub fn brain(&self) -> Arc<Brain> {
        Arc::clone(&self.brain)
    }
    pub fn registry(&self) -> Arc<ChipRegistry> {
        Arc::clone(&self.registry)
    }
    pub fn event_tx(&self) -> broadcast::Sender<CanonicalEvent> {
        self.event_tx.clone()
    }
    pub fn chip_tx(&self) -> broadcast::Sender<ActionChip> {
        self.chip_tx.clone()
    }

    /// Soket dizinini 0700 izinleriyle oluşturur ve bayat soketi temizler.
    fn prepare_socket(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
                let perms = fs::Permissions::from_mode(0o700);
                fs::set_permissions(parent, perms)?;
            }
        }
        if path.exists() {
            debug!("Bayat soket dosyası temizleniyor: {:?}", path);
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    /// Daemon ana dinleme döngüsünü başlatır.
    pub async fn run(
        self: Arc<Self>,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> std::io::Result<()> {
        Self::prepare_socket(&self.socket_path)?;

        let listener = UnixListener::bind(&self.socket_path)?;
        let perms = fs::Permissions::from_mode(0o700);
        let _ = fs::set_permissions(&self.socket_path, perms);

        info!("👁️ Thornhill Bus UDS dinlemede: {:?}", self.socket_path);

        loop {
            tokio::select! {
                Ok((stream, _)) = listener.accept() => {
                    let server = Arc::clone(&self);
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            debug!("Bağlantı kapandı: {}", e);
                        }
                    });
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Thornhill Bus kapatma sinyali alındı, soket temizleniyor...");
                        let _ = fs::remove_file(&self.socket_path);
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Tekil istemci bağlantısını işler; RFC-001 §8.4 (SO_PEERCRED) kontrolü uygular.
    async fn handle_connection(
        &self,
        stream: UnixStream,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let my_uid = unsafe { libc::getuid() };
        match stream.peer_cred() {
            Ok(cred) => {
                let peer_uid = cred.uid();
                if peer_uid != my_uid {
                    warn!(
                        "Güvenlik İhlali: Sokete yabancı UID bağlandı! Beklenen: {}, Gelen: {}",
                        my_uid, peer_uid
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                warn!("Kimlik doğrulama başarısız (SO_PEERCRED okunamadı): {}", e);
                return Ok(());
            }
        }

        let (read_half, write_half) = stream.into_split();
        let mut framed_read =
            FramedRead::new(read_half, LinesCodec::new_with_max_length(1024 * 1024));
        let mut framed_write =
            FramedWrite::new(write_half, LinesCodec::new_with_max_length(1024 * 1024));

        let mut sub_filter: Option<SubFilter> = None;
        let mut chip_streaming = false;
        let mut sub_rx = self.event_tx.subscribe();
        let mut chip_rx = self.chip_tx.subscribe();

        loop {
            tokio::select! {
                line_res = framed_read.next() => {
                    match line_res {
                        Some(Ok(line)) => {
                            let resp = self.dispatch_request(&line, &mut sub_filter).await;
                            chip_streaming = sub_filter
                                .as_ref()
                                .map(|f| f.chips)
                                .unwrap_or(false);
                            let resp_json = serde_json::to_string(&resp)?;
                            framed_write.send(resp_json).await?;
                        }
                        Some(Err(e)) => {
                            error!("NDJSON okuma hatası: {}", e);
                            break;
                        }
                        None => break,
                    }
                }

                res = sub_rx.recv(), if sub_filter.is_some() => {
                    match res {
                        Ok(event) => {
                            if let Some(ref filter) = sub_filter {
                                if filter.matches(&event) {
                                    let notification = WireResponse::event_push(event);
                                    let json = serde_json::to_string(&notification)?;
                                    if let Err(e) = framed_write.send(json).await {
                                        debug!("Aboneye bildirim gönderilemedi: {}", e);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Abone {} olayı kaçırdı (coalesce politikası uyarınca atıldı)", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }

                chip_res = chip_rx.recv(), if chip_streaming => {
                    match chip_res {
                        Ok(chip) => {
                            let notification = WireResponse::chip_push(chip);
                            let json = serde_json::to_string(&notification)?;
                            if let Err(e) = framed_write.send(json).await {
                                debug!("Çip bildirimi gönderilemedi: {}", e);
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Çip abonesi {} çipi kaçırdı (ttl zaten kısa ömürlü)", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }

        Ok(())
    }

    /// Gelen NDJSON isteğini ayrıştırır ve yanıtı üretir.
    async fn dispatch_request(
        &self,
        line: &str,
        sub_filter: &mut Option<SubFilter>,
    ) -> WireResponse {
        let req: WireRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                return WireResponse::err("", format!("Geçersiz NDJSON zarfı: {}", e));
            }
        };

        match req {
            WireRequest::Hello { req, client, v } => {
                debug!("İstemci tokalaşması: {} (v{})", client, v);
                WireResponse::ok(
                    req,
                    serde_json::json!({
                        "v": 1,
                        "daemon": "thornhilld",
                        "features": ["sensory", "graph", "chip", "s1_reflex", "actuator", "s2_brain", "stats", "escalate_log"],
                        "status": "ready"
                    }),
                )
            }
            WireRequest::Ping { req } => WireResponse::ok(req, serde_json::json!({ "pong": true })),
            WireRequest::Event { req, event } => {
                let id = event.id.clone();
                if !self.ingest(event) {
                    return WireResponse::err(req, "Olay kalıcılaştırılamadı");
                }
                WireResponse::ok(req, serde_json::json!({ "id": id }))
            }
            WireRequest::Sub { req, filter } => {
                debug!(
                    "Abone kaydı: kinds={:?}, srcs={:?}, chips={}",
                    filter.kinds, filter.srcs, filter.chips
                );
                *sub_filter = Some(filter);
                WireResponse::ok(req, serde_json::json!({ "subscribed": true }))
            }
            WireRequest::Query { req, q, limit } => match self.graph.query(&q, limit) {
                Ok(rows) => WireResponse::ok(req, serde_json::json!({ "rows": rows })),
                Err(e) => WireResponse::err(req, format!("Graf sorgusu hatası: {}", e)),
            },
            // Faz 5: denetim bulgusu #5 — stats artık canlıya bağlı.
            WireRequest::Stats { req } => match self.graph.stats() {
                Ok(s) => WireResponse::ok(req, s),
                Err(e) => WireResponse::err(req, format!("Stats hatası: {}", e)),
            },
            WireRequest::ChipAct {
                req,
                chip_id,
                confirm,
                action_label,
            } => {
                info!("ChipAct: {} (onay={})", chip_id, confirm);

                let chip = match self.registry.peek(&chip_id) {
                    Some(c) => c,
                    None => {
                        let _ = self.graph.append(&CanonicalEvent::new(
                            "user",
                            "chip.dismiss",
                            &chip_id,
                            serde_json::json!({"reason": "expired_or_unknown"}),
                        ));
                        return WireResponse::err(
                            req,
                            format!("Çip bulunamadı veya TTL doldu: {}", chip_id),
                        );
                    }
                };

                let action = match action_label
                    .as_ref()
                    .and_then(|label| chip.actions.iter().find(|a| &a.label == label))
                    .or_else(|| chip.actions.first())
                {
                    Some(a) => a.clone(),
                    None => return WireResponse::err(req, "Çipte yürütülebilir eylem yok"),
                };

                if !confirm {
                    use crate::actuator::ladder_check;
                    return match ladder_check(&action.risk, false) {
                        crate::actuator::Decision::Refused(reason) => {
                            WireResponse::err(req, format!("Reddedildi: {}", reason))
                        }
                        crate::actuator::Decision::NeedHoldConfirm => WireResponse::ok(
                            req,
                            serde_json::json!({
                                "dispatched": false,
                                "requires": "hold-to-confirm (confirm=true ile ikinci çağrı)",
                                "action": action.action_type, "risk": action.risk,
                            }),
                        ),
                        crate::actuator::Decision::Allow => WireResponse::ok(
                            req,
                            serde_json::json!({
                                "dispatched": false,
                                "requires": "confirm=true",
                                "action": action.action_type, "risk": action.risk,
                            }),
                        ),
                    };
                }

                match action.action_type.as_str() {
                    "ask_llm" => {
                        let question = action
                            .payload
                            .get("question")
                            .and_then(|q| q.as_str())
                            .unwrap_or("Bu hatayı açıkla")
                            .to_string();
                        match self.brain.answer_chip(&question).await {
                            Ok(ans) => {
                                let _ = self.graph.append(&CanonicalEvent::new(
                                    "user",
                                    "chip.accept",
                                    &chip_id,
                                    serde_json::json!({"action": "ask_llm"}),
                                ));
                                WireResponse::ok(
                                    req,
                                    serde_json::json!({
                                        "dispatched": true,
                                        "action": "ask_llm",
                                        "explanation": ans.explanation,
                                        "remedy": ans.remedy,
                                    }),
                                )
                            }
                            Err(e) => WireResponse::err(req, e),
                        }
                    }
                    _ => {
                        let outcome = execute_action(&action, &chip).await;
                        let _ = self.graph.append(&CanonicalEvent::new(
                            "user",
                            "chip.accept",
                            &chip_id,
                            serde_json::json!({"action": action.action_type}),
                        ));
                        self.registry.remove(&chip_id);

                        match outcome {
                            crate::actuator::Outcome::Exec {
                                exit_code,
                                stdout,
                                stderr,
                            } => WireResponse::ok(
                                req,
                                serde_json::json!({
                                    "dispatched": true, "action": action.action_type,
                                    "exit_code": exit_code, "stdout": stdout, "stderr": stderr,
                                }),
                            ),
                            crate::actuator::Outcome::DryRunOk { note } => WireResponse::ok(
                                req,
                                serde_json::json!({
                                    "dispatched": true, "action": action.action_type,
                                    "dry_run": true, "note": note,
                                }),
                            ),
                            crate::actuator::Outcome::Refused { reason } => {
                                WireResponse::err(req, format!("Reddedildi: {}", reason))
                            }
                        }
                    }
                }
            }
            WireRequest::Ask { req, question } => {
                info!("❓ S2 sorgusu alındı: {:?}", question);
                match self.brain.answer_query(&question).await {
                    Ok(ans) => {
                        let _ = self.graph.append(&CanonicalEvent::new(
                            "user",
                            "s2_query",
                            &question,
                            serde_json::json!({"confidence": ans.confidence}),
                        ));
                        WireResponse::ok(
                            req,
                            serde_json::json!({
                                "explanation": ans.explanation,
                                "remedy": ans.remedy,
                                "confidence": ans.confidence,
                            }),
                        )
                    }
                    Err(e) => WireResponse::err(req, e),
                }
            }
            WireRequest::Forget { req, before_ts } => match self.graph.forget_before(before_ts) {
                Ok((events, edges)) => WireResponse::ok(
                    req,
                    serde_json::json!({
                        "forgotten_events": events,
                        "forgotten_edges": edges,
                        "note": "lessons/habits korunur (RFC-001 §8.6)"
                    }),
                ),
                Err(e) => WireResponse::err(req, format!("Unutma hatası: {}", e)),
            },
        }
    }
}
