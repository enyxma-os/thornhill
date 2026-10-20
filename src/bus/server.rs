use crate::protocol::{CanonicalEvent, SubFilter, WireRequest, WireResponse};
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
        PathBuf::from(runtime_dir).join("thornhill").join("thornhill.sock")
    } else {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{}/thornhill/thornhill.sock", uid))
    }
}

pub struct BusServer {
    socket_path: PathBuf,
    event_tx: broadcast::Sender<CanonicalEvent>,
}

impl BusServer {
    pub fn new(socket_path: Option<PathBuf>) -> Self {
        let path = socket_path.unwrap_or_else(resolve_socket_path);
        let (event_tx, _) = broadcast::channel(1024);

        Self {
            socket_path: path,
            event_tx,
        }
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
    pub async fn run(self: Arc<Self>, mut shutdown_rx: tokio::sync::watch::Receiver<bool>) -> std::io::Result<()> {
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

    /// Tekil istemci bağlantısını işler ve RFC-001 §8.4 (SO_PEERCRED) kontrolünü uygular.
    async fn handle_connection(&self, stream: UnixStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // RFC-001 §8.4: SO_PEERCRED yetki kontrolü
        let my_uid = unsafe { libc::getuid() };
        match stream.peer_cred() {
            Ok(cred) => {
                let peer_uid = cred.uid();
                if peer_uid != my_uid {
                    warn!("Güvenlik İhlali: Sokete yabancı UID bağlandı! Beklenen: {}, Gelen: {}", my_uid, peer_uid);
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
        let mut sub_rx = self.event_tx.subscribe();

        loop {
            tokio::select! {
                // İstemciden gelen satırlar
                line_res = framed_read.next() => {
                    match line_res {
                        Some(Ok(line)) => {
                            let resp = self.dispatch_request(&line, &mut sub_filter).await;
                            let resp_json = serde_json::to_string(&resp)?;
                            framed_write.send(resp_json).await?;
                        }
                        Some(Err(e)) => {
                            error!("NDJSON okuma hatası: {}", e);
                            break;
                        }
                        None => break, // EOF
                    }
                }

                // Abone olunan olayları istemciye fırlat (EventNotification / ChipPush)
                Ok(event) = sub_rx.recv(), if sub_filter.is_some() => {
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
            }
        }

        Ok(())
    }

    /// Gelen NDJSON isteğini ayrıştırır ve uygun yanıtı üretir.
    async fn dispatch_request(&self, line: &str, sub_filter: &mut Option<SubFilter>) -> WireResponse {
        let req: WireRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                return WireResponse::err("", format!("Geçersiz NDJSON zarfı: {}", e));
            }
        };

        match req {
            WireRequest::Hello { req, client, v } => {
                debug!("İstemci tokalaşması: {} (v{})", client, v);
                WireResponse::ok(req, serde_json::json!({
                    "v": 1,
                    "daemon": "thornhilld",
                    "features": ["sensory", "graph", "chip", "s1_reflex"],
                    "status": "ready"
                }))
            }
            WireRequest::Ping { req } => {
                WireResponse::ok(req, serde_json::json!({ "pong": true }))
            }
            WireRequest::Event { req, event } => {
                let id = event.id.clone();
                debug!("Kanonik olay alındı: {}/{} [{}]", event.src, event.kind, id);
                // Canlı abonelere ilet
                let _ = self.event_tx.send(event);
                WireResponse::ok(req, serde_json::json!({ "id": id }))
            }
            WireRequest::Sub { req, filter } => {
                debug!("Abone kaydı yapıldı: kinds={:?}, srcs={:?}", filter.kinds, filter.srcs);
                *sub_filter = Some(filter);
                WireResponse::ok(req, serde_json::json!({ "subscribed": true }))
            }
            WireRequest::Query { req, sql } => {
                // SQLite grafiği fazında burası doğrudan graf motoruna bağlanacak
                WireResponse::ok(req, serde_json::json!({ "rows": [], "query": sql }))
            }
            WireRequest::ChipAct { req, chip_id, confirm } => {
                info!("Action Chip eylemi alındı: {} (onay={})", chip_id, confirm);
                WireResponse::ok(req, serde_json::json!({ "dispatched": true, "chip_id": chip_id }))
            }
        }
    }
}
