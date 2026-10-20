use crate::actuator::{execute_action, ChipRegistry};
use crate::graph::GraphStore;
use crate::protocol::{ActionChip, CanonicalEvent, SubFilter, WireRequest, WireResponse};
use crate::reflex::ReflexEngine;
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

/// §6.3 korelasyon penceresi: kenar kurmak için olaylar bu kadar yakınsa
/// "caused" adayıdır (RFC-001 §6.3 deterministik kural seti).
const CORRELATION_WINDOW_MS: u64 = 10_000;

pub struct BusServer {
    socket_path: PathBuf,
    event_tx: broadcast::Sender<CanonicalEvent>,
    /// S1 çip kanalı (§7.2): broadcast — paylaşımlı kilit yok.
    chip_tx: broadcast::Sender<ActionChip>,
    graph: Arc<GraphStore>,
    reflex: Arc<ReflexEngine>,
    /// Faz 2.1: canlı çip kayıt defteri (TTL + onay doğrulaması).
    registry: Arc<ChipRegistry>,
}

impl BusServer {
    pub fn new(socket_path: Option<PathBuf>, graph: Arc<GraphStore>) -> Self {
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
        }
    }

    /// Duyu modülleri için içsel besleme yolu (Faz 1 Adım 4).
    pub fn ingest(&self, event: CanonicalEvent) -> bool {
        // 1) Olayı önce kalıcılaştır (tek gerçek kaynak; edges için foreign key şartı)
        if let Err(e) = self.graph.append(&event) {
            error!("Duyu olayı grafe yazılamadı: {}", e);
            return false;
        }
        let _ = self.event_tx.send(event.clone());

        // 2) §6.3 Adım A: Penceredeki önceki olaylarla deterministik korelasyon.
        // git.dirty / fs.write → cmd.done{exit≠0} zinciri "caused" kenarı kurar.
        if event.src == "shell" && event.kind == "cmd.done" {
            let failed = event
                .ctx
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .map(|c| c != 0)
                .unwrap_or(false);
            if failed {
                match self.graph.recent_within(CORRELATION_WINDOW_MS) {
                    Ok(prior) => {
                        for (pid, kind, _subject) in &prior {
                            if pid == &event.id {
                                continue;
                            }
                            let relevant = matches!(
                                kind.as_str(),
                                "git.dirty" | "git.commit" | "fs.write" | "proc.exec"
                            );
                            if relevant {
                                match self.graph.link(pid, &event.id, "caused", 0.5) {
                                    Ok(support) => {
                                        debug!(
                                            "§6.3 kenar: {} → {} (support={})",
                                            kind, event.kind, support
                                        );
                                        // support >= 3 → lesson'a yüksel (§6.3 güçlendirme)
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
                                    Err(e) => debug!("Kenar yazılamadı: {}", e),
                                }
                            }
                        }
                    }
                    Err(e) => debug!("Korelasyon penceresi okunamadı: {}", e),
                }
            }
        }

        // 3) S1 Refleks (§9.1): LLM'siz, <1 ms.
        if let Some(mut chip) = self.reflex.evaluate(&event) {
            debug!("S1 çip üretti: {} ({})", chip.id, chip.title);
            // S1 chip.why'sını §6.3 kenar bilgisiyle zenginleştir (nedensel gerekçe)
            if let Ok(rows) = self.graph.query("causal", 5) {
                if !rows.is_empty() {
                    chip.why = format!("{} | graf: {} kenar destekli", chip.why, rows.len());
                }
            }
            self.registry.insert(chip.clone());
            let _ = self.chip_tx.send(chip);
        }
        true
    }
    /// Soket dizinini 0700 izinleriyle oluşturur ve bayat soketi temizler.
    fn prepare_socket(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
                let perms = fs::Permissions::from_mode(0o700);
                fs::set_permissions(parent, perms)?;
                debug!("Soket dizini 0700 izinleriyle oluşturuldu: {:?}", parent);
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
        let mut framed_read = FramedRead::new(read_half, LinesCodec::new());
        let mut framed_write = FramedWrite::new(write_half, LinesCodec::new());

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
                        "features": ["sensory", "graph", "chip", "s1_reflex", "actuator"],
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
                debug!("Kanonik olay işlendi (graf + yayın + S1 + §6.3): {}", id);
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
            // Faz 2.1: Gerçek §8.5 dağıtım.
            WireRequest::ChipAct {
                req,
                chip_id,
                confirm,
                action_label,
            } => {
                info!("ChipAct: {} (onay={})", chip_id, confirm);

                // 1) Çip canlı mı? (TTL §7.2)
                let chip = match self.registry.peek(&chip_id) {
                    Some(c) => c,
                    None => {
                        // Dismiss grafe yazılır (§7.2) — model öğrenir.
                        let _ = self.graph.append(&CanonicalEvent::new(
                            "user",
                            "chip.dismiss",
                            &chip_id,
                            serde_json::json!({"reason":"expired_or_unknown"}),
                        ));
                        return WireResponse::err(
                            req,
                            format!("Çip bulunamadı veya TTL doldu: {}", chip_id),
                        );
                    }
                };

                // 2) Eylemi seç (action_label yoksa ilk)
                let action = match action_label
                    .as_ref()
                    .and_then(|label| chip.actions.iter().find(|a| &a.label == label))
                    .or_else(|| chip.actions.first())
                {
                    Some(a) => a.clone(),
                    None => {
                        return WireResponse::err(req, "Çipte yürütülebilir eylem yok");
                    }
                };

                // 3) §8.5 merdiven kararı
                if confirm {
                    let outcome = execute_action(&action, &chip).await;
                    // Denetim olayı: her yürütme grafe yazılır (§8.5)
                    let (kind, ctx) = match &outcome {
                        crate::actuator::Outcome::Exec { exit_code, .. } => (
                            "chip.accept",
                            serde_json::json!({"action": action.action_type, "exit_code": exit_code}),
                        ),
                        other => (
                            "chip.accept",
                            serde_json::json!({"action": action.action_type, "outcome": format!("{:?}", other)}),
                        ),
                    };
                    let _ = self
                        .graph
                        .append(&CanonicalEvent::new("user", kind, &chip_id, ctx));
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
                                "dispatched": true, "action": action.action_type, "dry_run": true, "note": note,
                            }),
                        ),
                        crate::actuator::Outcome::Stub { note } => WireResponse::ok(
                            req,
                            serde_json::json!({
                                "dispatched": false, "action": action.action_type, "note": note,
                            }),
                        ),
                        crate::actuator::Outcome::Refused { reason } => {
                            WireResponse::err(req, format!("Reddedildi: {}", reason))
                        }
                    }
                } else {
                    // confirm=false: yalnız merdiven önizlemesi — hiçbir şey çalışmaz.
                    use crate::actuator::ladder_check;
                    match ladder_check(&action.risk, false) {
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
                    }
                }
            }
        }
    }
}
