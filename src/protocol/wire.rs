use crate::protocol::event::CanonicalEvent;
use serde::{Deserialize, Serialize};

/// RFC-001 §7.1: İstemci -> Daemon İstekleri
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WireRequest {
    Hello {
        #[serde(default)]
        req: String,
        client: String,
        #[serde(default = "default_version")]
        v: u8,
    },
    Ping {
        #[serde(default)]
        req: String,
    },
    Event {
        #[serde(default)]
        req: String,
        event: CanonicalEvent,
    },
    Sub {
        #[serde(default)]
        req: String,
        #[serde(default)]
        filter: SubFilter,
    },
    /// RFC-001 §7.1: Yapılandırılmış graf sorgusu (salt-okunur).
    Query {
        #[serde(default)]
        req: String,
        q: String,
        #[serde(default = "default_limit")]
        limit: u32,
    },
    /// RFC-001 §7.2: Action Chip onay/ret
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

fn default_limit() -> u32 {
    100
}

/// Abonelik filtre kriterleri
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SubFilter {
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub srcs: Vec<String>,
    /// S1 çip akışını da (chip.push) ister (RFC-001 §7.2)
    #[serde(default)]
    pub chips: bool,
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

/// RFC-001 §7.2: Chip içindeki tekil eylem
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChipAction {
    pub label: String,
    /// ask_llm | exec | apply_patch | config_write | open
    pub action_type: String,
    /// RFC-001 §8.5 risk merdiveni: none | low | medium | high
    pub risk: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// RFC-001 §7.2: Action Chip mesajı
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionChip {
    pub id: String,
    /// Kuralın tetik açıklaması (insan-okunur)
    pub trigger: String,
    pub title: String,
    /// Neden bu çip basıldı (nedensel gerekçe)
    pub why: String,
    pub labels: Vec<String>,
    pub actions: Vec<ChipAction>,
    /// Bu süre içinde onaylanmazsa otomatik düşer (dismiss grafe yazılır)
    pub ttl_ms: u64,
    pub conf: f64,
}

/// RFC-001 §7.1 & §7.2: Daemon -> İstemci Yanıtları
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum WireResponse {
    Success {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        req: Option<String>,
        #[serde(flatten)]
        data: serde_json::Value,
    },
    Error {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        req: Option<String>,
        error: String,
    },
    /// Canlı olay akışı bildirimi
    EventNotification { op: String, event: CanonicalEvent },
    /// Canlı Action Chip bildirimi (§7.2)
    ChipPush { op: String, chip: ActionChip },
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

    pub fn chip_push(chip: ActionChip) -> Self {
        Self::ChipPush {
            op: "chip.push".to_string(),
            chip,
        }
    }
}
