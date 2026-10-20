use crate::bus::server::resolve_socket_path;
use crate::protocol::{SubFilter, WireRequest, WireResponse};
use futures_util::{SinkExt, StreamExt};
use std::path::Path;
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub struct BusClient {
    read_half: FramedRead<tokio::net::unix::OwnedReadHalf, LinesCodec>,
    write_half: FramedWrite<tokio::net::unix::OwnedWriteHalf, LinesCodec>,
}

impl BusClient {
    /// UDS soketine bağlanır.
    pub async fn connect(
        socket_path: Option<&Path>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = match socket_path {
            Some(p) => p.to_path_buf(),
            None => resolve_socket_path(),
        };

        if !path.exists() {
            return Err(format!(
                "Thornhill UDS soketi bulunamadı: {:?}. Daemon çalışıyor mu?",
                path
            )
            .into());
        }

        let stream = UnixStream::connect(path).await?;
        let (r, w) = stream.into_split();
        let read_half = FramedRead::new(r, LinesCodec::new());
        let write_half = FramedWrite::new(w, LinesCodec::new());

        Ok(Self {
            read_half,
            write_half,
        })
    }

    /// Tekil bir istek gönderir ve yanıtı bekler.
    pub async fn call(
        &mut self,
        req: &WireRequest,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let req_json = serde_json::to_string(req)?;
        self.write_half.send(req_json).await?;

        if let Some(line_res) = self.read_half.next().await {
            let line = line_res?;
            let val: serde_json::Value = serde_json::from_str(&line)?;
            if val.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                let err_msg = val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Bilinmeyen hata");
                return Err(err_msg.into());
            }
            Ok(val)
        } else {
            Err("Sunucudan yanıt alınamadı (EOF)".into())
        }
    }

    /// Olay akışına abone olur ve dinleyici akışı başlatır.
    pub async fn subscribe(
        &mut self,
        filter: SubFilter,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let req = WireRequest::Sub {
            req: "sub_1".to_string(),
            filter,
        };
        let _ = self.call(&req).await?;
        Ok(())
    }

    /// Bir sonraki akış bildirimini bekler.
    pub async fn next_message(
        &mut self,
    ) -> Option<Result<WireResponse, Box<dyn std::error::Error + Send + Sync>>> {
        match self.read_half.next().await {
            Some(Ok(line)) => match serde_json::from_str::<WireResponse>(&line) {
                Ok(resp) => Some(Ok(resp)),
                Err(e) => Some(Err(format!("Ayrıştırma hatası: {}", e).into())),
            },
            Some(Err(e)) => Some(Err(Box::new(e))),
            None => None,
        }
    }
}
