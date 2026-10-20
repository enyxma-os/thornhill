//! RFC-001 §9.1 — Sistem 1 (Refleks Çekirdeği).
//!
//! Katı kurallar (RFC-001 §9.1):
//! - LLM çağrısı YOKTUR; VRAM kullanımı 0.
//! - Yanıt bütçesi: p99 < 1 ms (G3). `evaluate()` bunu kendi içinde ölçer.
//! - Kurallar deterministik: (src, kind) + ctx predicate -> Action Chip (§7.2).

use crate::protocol::wire::ActionChip;
use std::time::{Duration, Instant};
use tracing::debug;

pub struct ReflexEngine;

impl ReflexEngine {
    pub fn new() -> Self {
        Self
    }

    /// S1 değerlendirme girişi. Bütçeyi ölçer, aşarsa uyarır (G3 regisyon izi).
    pub fn evaluate(&self, event: &crate::protocol::CanonicalEvent) -> Option<ActionChip> {
        let start = Instant::now();
        let chip = self.evaluate_inner(event);
        let elapsed = start.elapsed();

        if elapsed > Duration::from_millis(1) {
            tracing::warn!(
                "⚠️ S1 bütçe ihlali: değerlendirme {:?} sürdü (bütçe <1 ms)",
                elapsed
            );
        } else {
            debug!("S1 değerlendirme: {:?} (bütçe içinde)", elapsed);
        }
        chip
    }

    fn evaluate_inner(&self, ev: &crate::protocol::CanonicalEvent) -> Option<ActionChip> {
        match (ev.src.as_str(), ev.kind.as_str()) {
            // KURAL 1: Başarısız komut -> hata açıklama + yeniden çalıştır çipi
            ("shell", "cmd.done") => {
                let exit_code = ev.ctx.get("exit_code")?.as_i64()?;
                if exit_code == 0 {
                    return None;
                }
                let cmd = ev
                    .ctx
                    .get("cmd")
                    .and_then(|v| v.as_str())
                    .unwrap_or("bilinmeyen komut");
                let err = ev.ctx.get("error").and_then(|v| v.as_str()).unwrap_or("");

                Some(ActionChip {
                    id: format!("chip_{}", ulid::Ulid::new()),
                    trigger: format!("cmd.done{{exit={}, cmd~'{}'}}", exit_code, cmd),
                    title: format!("Komut hatası (exit {}): {}", exit_code, cmd),
                    why: if err.is_empty() {
                        "S1 kuralı: sıfırdan farklı çıkış kodu".to_string()
                    } else {
                        format!("Hata çıktısı: {}", err)
                    },
                    labels: vec![
                        "Hatayı Açıkla".to_string(),
                        "Komutu Yeniden Çalıştır".to_string(),
                    ],
                    actions: vec![
                        crate::protocol::wire::ChipAction {
                            label: "Hatayı Açıkla".to_string(),
                            action_type: "ask_llm".to_string(),
                            risk: "none".to_string(),
                            payload: serde_json::json!({
                                "question": format!(
                                    "Bu komut neden başarısız oldu (exit {})? Hata: {}",
                                    exit_code, err
                                )
                            }),
                        },
                        crate::protocol::wire::ChipAction {
                            label: "Komutu Yeniden Çalıştır".to_string(),
                            action_type: "exec".to_string(),
                            risk: "medium".to_string(),
                            payload: serde_json::json!({ "cmd": cmd, "confirm": "hold" }),
                        },
                    ],
                    ttl_ms: 60_000,
                    conf: 1.0,
                })
            }
            // KURAL 2 (vadesi Faz 3): habit/lesson eşleşmesiyle genişlenecek
            // (RFC-001 §6.3 destek mekanizması bağlandığında).
            _ => None,
        }
    }
}

impl Default for ReflexEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CanonicalEvent;
    use serde_json::json;

    fn shell_event(exit_code: i64, cmd: &str, err: &str) -> CanonicalEvent {
        CanonicalEvent::new(
            "shell",
            "cmd.done",
            "12345",
            json!({ "exit_code": exit_code, "cmd": cmd, "error": err }),
        )
    }

    #[test]
    fn fires_on_failed_command() {
        let engine = ReflexEngine::new();
        let chip = engine
            .evaluate(&shell_event(101, "cargo build", "E0432 unresolved import"))
            .expect("chip üretilmeli");
        assert!(chip.id.starts_with("chip_"));
        assert_eq!(chip.labels.len(), 2);
        assert_eq!(chip.labels[0], "Hatayı Açıkla");
        assert_eq!(chip.actions[0].action_type, "ask_llm");
        assert_eq!(chip.actions[1].risk, "medium");
        assert_eq!(chip.conf, 1.0);
    }

    #[test]
    fn silent_on_success() {
        let engine = ReflexEngine::new();
        assert!(engine
            .evaluate(&shell_event(0, "cargo build", ""))
            .is_none());
    }

    #[test]
    fn silent_on_other_sources() {
        let engine = ReflexEngine::new();
        let ev = CanonicalEvent::new("hyprland", "win.focus", "kitty", json!({}));
        assert!(engine.evaluate(&ev).is_none());
    }

    #[test]
    fn under_1ms_budget() {
        // G3: <1 ms yanıt bütçesi. 1000 değerlendirme toplamı 1 sn'yi geçmemeli.
        let engine = ReflexEngine::new();
        let start = Instant::now();
        for _ in 0..1000 {
            let _ = engine.evaluate(&shell_event(1, "false", ""));
        }
        let per_call = start.elapsed() / 1000;
        assert!(
            per_call < Duration::from_millis(1),
            "Ortalama çağrı bütçeyi aştı: {:?}",
            per_call
        );
    }

    #[test]
    fn missing_ctx_is_safe() {
        // ctx'te exit_code yoksa kural patlamamalı (None dönmeli).
        let engine = ReflexEngine::new();
        let ev = CanonicalEvent::new("shell", "cmd.done", "1", json!({}));
        assert!(engine.evaluate(&ev).is_none());
    }
}
