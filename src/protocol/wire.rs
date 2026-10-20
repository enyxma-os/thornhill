use crate::protocol::event::CanonicalEvent;
use serde::{Deserialize, Serialize};

/// RFC-001 §7.1: İstemci -> Daemon İstekleri
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WireRequest {
    /// Bağlantı tokalaşması
    Hello {
        #[serde(default)]
        req: String,
        client: String,
        #[serde(default = "default_version")]
        v: u8,
    },
    /// Canlılık yoklaması
    Ping {
        #[serde(default)]
        req: String,
    },
    /// Duyu katmanından veya istemciden olay gönderimi
    Event {
        #[serde(default)]
        req: String,
        event: CanonicalEvent,
    },
    /// Olay veya çip akışına abone olma
    Sub {
        #[serde(default)]
        req: String,
        #[serde(default)]
        filter: SubFilter,
    },
    /// Doğrudan graf sorgusu (salt-okunur SQL)
    Query {
        #[serde(default)]
        req: String,
        sql: String,
    },
    /// Action Chip onay/ret eylemi
    #[serde(rename = "chip.act")]
    ChipAct {
        #[serde(default)]
        req: String,
        chip_id: String,
        confirm: bool,
    },
}

fn default_version() -> u8 {
    1
}

/// Abonelik filtre kriterleri
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SubFilter {
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub srcs: Vec<String>,
}

impl SubFilter {
    pub fn matches(&self, event: &CanonicalEvent) -> bool {
        let kind_ok = self.kinds.is_empty()
            || self.kinds.iter().any(|k| {
                if k.ends_with('*') {
                    event.kind.starts_with(&k[..k.len() - 1])
                } else {
                    event.kind == *k
                }
            });

        let src_ok = self.srcs.is_empty() || self.srcs.contains(&event.src);

        kind_ok && src_ok
    }
}

/// RFC-001 §7.1 & §7.2: Daemon -> İstemci Yanıtları
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum WireResponse {
    /// Başarılı genel yanıt (ack, hello_ack, ping, query sonucu vb.)
    Success {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        req: Option<String>,
        #[serde(flatten)]
        data: serde_json::Value,
    },
    /// Hata yanıtı
    Error {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        req: Option<String>,
        error: String,
    },
    /// Canlı olay akışı bildirimi
    EventNotification {
        op: String,
        event: CanonicalEvent,
    },
}

impl WireResponse {
    pub fn ok(req: impl Into<String>, data: serde_json::Value) -> Self {
        Self::Success {
            ok: true,
            req: Some(req.into()),
            data,
        }
    }

    pub fn err(req: impl Into<String>, error: impl Into<String>) -> Self {
        Self::Error {
            ok: false,
            req: Some(req.into()),
            error: error.into(),
        }
    }

    pub fn event_push(event: CanonicalEvent) -> Self {
        Self::EventNotification {
            op: "event.push".to_string(),
            event,
        }
    }
}
