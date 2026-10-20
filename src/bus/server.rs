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

pub struct BusServer {
    socket_path: PathBuf,
    event_tx: broadcast::Sender<CanonicalEvent>,
    /// S1 çip kanalı (§7.2): broadcast — her abone kendi Lagged takibini yapar,
    /// paylaşımlı Mutex kilidi YOK (çoklu Shell bağlantısı birbirini bloklamaz).
    chip_tx: broadcast::Sender<ActionChip>,
    graph: Arc<GraphStore>,
    reflex: Arc<ReflexEngine>,
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
        }
    }

    /// Duyu modülleri için içsel besleme yolu (Faz 1 Adım 4).
    /// Kalıcılaştırır, canlı abonelere yayınlar ve S1'i tetikler.
    pub fn ingest(&self, event: CanonicalEvent) -> bool {
        if let Err(e) = self.graph.append(&event) {
            error!("Duyu olayı grafe yazılamadı: {}", e);
            return false;
        }
        let _ = self.event_tx.send(event.clone());

        // S1 Refleks (§9.1): LLM'siz, <1 ms. Üretilen çip §7.2 kanalına düşer.
        if let Some(chip) = self.reflex.evaluate(&event) {
            debug!("S1 çip üretti: {} ({})", chip.id, chip.title);
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
        // RFC-001 §8.4: SO_PEERCRED yetki kontrolü
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
                            // Sub yanıtından sonra chip akış isteğini tazele
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

                // Olay akışı (Lagged meşgul döngüsü üretmez — Adım 3 düzeltmesi)
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

                // S1 çip akışı (§7.2) — yalnız chips=true aboneliğinde
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
                        "features": ["sensory", "graph", "chip", "s1_reflex"],
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
                debug!(
                    "Kanonik olay grafe yazıldı, yayınlandı ve S1'e verildi: {}",
                    id
                );
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
            // RFC-001 §7.1: yapılandırılmış sorgu; ham SQL tel üzerinden kabul edilmez.
            WireRequest::Query { req, q, limit } => match self.graph.query(&q, limit) {
                Ok(rows) => WireResponse::ok(req, serde_json::json!({ "rows": rows })),
                Err(e) => WireResponse::err(req, format!("Graf sorgusu hatası: {}", e)),
            },
            WireRequest::ChipAct {
                req,
                chip_id,
                confirm,
            } => {
                info!("Action Chip eylemi: {} (onay={})", chip_id, confirm);
                // TODO(Faz 2.5): risk merdivenine göre gerçek dağıtım (exec/apply_patch).
                // Şimdilik yalnızca kabul ve log — hiçbir eylem çalıştırılmaz (§8.5).
                WireResponse::ok(
                    req,
                    serde_json::json!({
                        "dispatched": false,
                        "chip_id": chip_id,
                        "note": "exec dağıtımı Faz 2.5'te; şu an yalnız kabul+log"
                    }),
                )
            }
        }
    }
}
