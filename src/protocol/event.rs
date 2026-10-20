use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

/// RFC-001 §5: Kanonik Olay Şeması (Canonical Event Schema)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalEvent {
    /// Şema sürümü (v1)
    #[serde(default = "default_version")]
    pub v: u8,

    /// ULID (zaman sıralı benzersiz kimlik)
    pub id: String,

    /// Zaman damgası (epoch milisaniye)
    pub ts: u64,

    /// Olay kaynağı (hyprland | inotify | git | proc | shell | udev | user)
    pub src: String,

    /// Olay türü (örn: cmd.done, win.focus, repo.enter)
    pub kind: String,

    /// Olay öznesi (pid, yol, aygıt, komut)
    pub subject: String,

    /// Kaynağa özel bağlam (JSON nesnesi)
    #[serde(default)]
    pub ctx: serde_json::Value,

    /// Güven skoru (0.0 - 1.0, duyu için varsayılan 1.0)
    #[serde(default = "default_confidence")]
    pub conf: f64,

    /// Opsiyonel: Önceki ilişkili olay kimliği (zincirleme için)
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
}

fn default_version() -> u8 {
    1
}

fn default_confidence() -> f64 {
    1.0
}

impl CanonicalEvent {
    /// Yeni bir kanonik olay oluşturur; ULID ve zaman damgasını otomatik üretir.
    pub fn new(
        src: impl Into<String>,
        kind: impl Into<String>,
        subject: impl Into<String>,
        ctx: serde_json::Value,
    ) -> Self {
        let ulid = Ulid::new();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Self {
            v: 1,
            id: ulid.to_string(),
            ts,
            src: src.into(),
            kind: kind.into(),
            subject: subject.into(),
            ctx,
            conf: 1.0,
            ref_id: None,
        }
    }

    /// Güven skorunu günceller.
    pub fn with_confidence(mut self, conf: f64) -> Self {
        self.conf = conf.clamp(0.0, 1.0);
        self
    }

    /// Zincirleme referans olayı ekler.
    pub fn with_ref(mut self, ref_id: impl Into<String>) -> Self {
        self.ref_id = Some(ref_id.into());
        self
    }
}
