//! RFC-001 §8.5 — Eylem Onay Merdiveni ve §7.2 çip kayıt defteri.
//!
//! Katı kurallar:
//! - `high` risk v1'de YASAK.
//! - `medium` (exec) yalnızca `confirm=true` ile çalışır; 10 sn timeout.
//! - `low` (apply_patch) v1'de yalnız dry-run (`git apply --check`).
//! - `none` (ask_llm) S2 Brain katmanı tarafından doğrudan yönetilir.
//! - Her yürütme denetim olayı (`user.chip.accept`) olarak grafe yazılır.

use crate::protocol::{ActionChip, ChipAction};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::timeout;
use tracing::info;

/// Canlı çip kaydı: TTL (§7.2) dolan çipler otomatik düşer.
pub struct ChipRegistry {
    entries: Mutex<HashMap<String, RegisteredChip>>,
}

struct RegisteredChip {
    chip: ActionChip,
    deadline: std::time::Instant,
}

impl ChipRegistry {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Çipi kaydeder; ttl_ms süresi kadar yaşar.
    pub fn insert(&self, chip: ActionChip) {
        let ttl = Duration::from_millis(chip.ttl_ms);
        let mut entries = self.entries.lock().expect("registry mutex");
        entries.insert(
            chip.id.clone(),
            RegisteredChip {
                chip,
                deadline: std::time::Instant::now() + ttl,
            },
        );
    }

    /// Çibi okur (silmeksizin). Süresi dolmuşsa temizler ve None döner.
    pub fn peek(&self, id: &str) -> Option<ActionChip> {
        let mut entries = self.entries.lock().expect("registry mutex");
        let now = std::time::Instant::now();
        // Süresi geçenleri süpür
        entries.retain(|_, r| r.deadline > now);
        entries.get(id).map(|r| r.chip.clone())
    }

    /// Çipi kaldırır (onaylandıktan/ reddedildikten sonra).
    pub fn remove(&self, id: &str) -> Option<ActionChip> {
        let mut entries = self.entries.lock().expect("registry mutex");
        entries.remove(id).map(|r| r.chip)
    }

    pub fn len(&self) -> usize {
        self.entries.lock().expect("registry mutex").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ChipRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// §8.5 merdiven kararı (saf fonksiyon — birim testleri burada).
pub enum Decision {
    /// Yürütülebilir
    Allow,
    /// İkinci onay gerekir (hold-to-confirm; CLI'da confirm=true ikinci çağrı)
    NeedHoldConfirm,
    /// Reddedildi (gerekçe ile)
    Refused(&'static str),
}

pub fn ladder_check(risk: &str, confirm: bool) -> Decision {
    match risk {
        "none" => Decision::Allow,
        "low" | "medium" => {
            if confirm {
                Decision::Allow
            } else {
                Decision::NeedHoldConfirm
            }
        }
        "high" => Decision::Refused("high risk v1'de yasak (RFC-001 §8.5)"),
        _other => Decision::Refused("bilinmeyen risk"),
    }
}

/// Yürütme sonucu — yanıtta istemciye olduğu gibi döner.
#[derive(Debug)]
pub enum Outcome {
    Exec {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    DryRunOk {
        note: String,
    },
    Refused {
        reason: String,
    },
}

const EXEC_TIMEOUT: Duration = Duration::from_secs(10);

/// Tek çip eylemini §8.5 kurallarına göre yürütür.
pub async fn execute_action(action: &ChipAction, _chip: &ActionChip) -> Outcome {
    if let Decision::Refused(reason) = ladder_check(&action.risk, true) {
        return Outcome::Refused {
            reason: reason.to_string(),
        };
    }

    match action.action_type.as_str() {
        "exec" => {
            let cmd = action
                .payload
                .get("cmd")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if cmd.is_empty() {
                return Outcome::Refused {
                    reason: "exec eyleminde cmd boş".into(),
                };
            }
            info!("⚡ Eylem yürütülüyor (medium/confirm): {:?}", cmd);
            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .output();
            match timeout(EXEC_TIMEOUT, child).await {
                Ok(Ok(out)) => Outcome::Exec {
                    exit_code: out.status.code().unwrap_or(-1),
                    stdout: String::from_utf8_lossy(&out.stdout)
                        .chars()
                        .take(2000)
                        .collect(),
                    stderr: String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(2000)
                        .collect(),
                },
                Ok(Err(e)) => Outcome::Refused {
                    reason: format!("süreç başlatılamadı: {}", e),
                },
                Err(_) => Outcome::Refused {
                    reason: "10 sn timeout aşıldı".into(),
                },
            }
        }
        "apply_patch" => {
            // §8.5 low: dry-run zorunlu — v1 yalnız doğrulama yapar, uygulamaz.
            let diff = action
                .payload
                .get("diff")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if diff.is_empty() {
                return Outcome::Refused {
                    reason: "apply_patch payload'ında diff yok".into(),
                };
            }
            // git apply --check ile uygulanabilirlik testi (yazma YOK)
            let child = tokio::process::Command::new("git")
                .args(["apply", "--check", "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();
            match child {
                Ok(mut proc) => {
                    use tokio::io::AsyncWriteExt;
                    if let Some(mut stdin) = proc.stdin.take() {
                        let _ = stdin.write_all(diff.as_bytes()).await;
                    }
                    match timeout(EXEC_TIMEOUT, proc.wait_with_output()).await {
                        Ok(Ok(out)) if out.status.success() => Outcome::DryRunOk {
                            note: "diff uygulanabilir (dry-run geçti); gerçek uygulama ikinci onay ister".into(),
                        },
                        Ok(Ok(out)) => Outcome::Refused {
                            reason: format!(
                                "dry-run başarısız: {}",
                                String::from_utf8_lossy(&out.stderr)
                            ),
                        },
                        _ => Outcome::Refused {
                            reason: "dry-run timeout".into(),
                        },
                    }
                }
                Err(e) => Outcome::Refused {
                    reason: format!("git başlatılamadı: {}", e),
                },
            }
        }

        "copy" => {
            let text = action
                .payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if text.is_empty() {
                return Outcome::Refused {
                    reason: "copy eyleminde text boş".into(),
                };
            }
            let child = tokio::process::Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn();
            match child {
                Ok(mut proc) => {
                    use tokio::io::AsyncWriteExt;
                    if let Some(mut stdin) = proc.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes()).await;
                    }
                    match timeout(EXEC_TIMEOUT, proc.wait_with_output()).await {
                        Ok(Ok(out)) if out.status.success() => Outcome::DryRunOk {
                            note: "Panoya kopyalandı".into(),
                        },
                        Ok(Ok(out)) => Outcome::Refused {
                            reason: format!(
                                "wl-copy hatası: {}",
                                String::from_utf8_lossy(&out.stderr)
                            ),
                        },
                        _ => Outcome::Refused {
                            reason: "wl-copy timeout".into(),
                        },
                    }
                }
                Err(e) => Outcome::Refused {
                    reason: format!("wl-copy başlatılamadı: {}", e),
                },
            }
        }
        other => Outcome::Refused {
            reason: format!("bilinmeyen eylem türü: {}", other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chip(ttl_ms: u64) -> ActionChip {
        ActionChip {
            id: format!("chip_{}", ulid::Ulid::new()),
            trigger: "test".into(),
            title: "Test çipi".into(),
            why: "test".into(),
            labels: vec!["A".into()],
            actions: vec![],
            ttl_ms,
            conf: 1.0,
        }
    }

    #[test]
    fn registry_expires_by_ttl() {
        let reg = ChipRegistry::new();
        let c = chip(0); // anında düşer
        reg.insert(c.clone());
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(reg.peek(&c.id).is_none(), "TTL dolan çip düşmeli");
    }

    #[test]
    fn registry_peek_keeps_alive() {
        let reg = ChipRegistry::new();
        let c = chip(60_000);
        reg.insert(c.clone());
        assert!(reg.peek(&c.id).is_some());
        assert!(reg.remove(&c.id).is_some());
        assert!(reg.peek(&c.id).is_none());
    }

    #[test]
    fn ladder_refuses_high_always() {
        assert!(matches!(ladder_check("high", true), Decision::Refused(_)));
        assert!(matches!(ladder_check("high", false), Decision::Refused(_)));
    }

    #[test]
    fn ladder_requires_confirm_for_medium() {
        assert!(matches!(
            ladder_check("medium", false),
            Decision::NeedHoldConfirm
        ));
        assert!(matches!(ladder_check("medium", true), Decision::Allow));
        assert!(matches!(ladder_check("low", true), Decision::Allow));
    }

    #[test]
    fn ladder_allows_none() {
        assert!(matches!(ladder_check("none", false), Decision::Allow));
    }

    #[tokio::test]
    async fn copy_refuses_empty_text() {
        let action = ChipAction {
            label: "Kopyala".into(),
            action_type: "copy".into(),
            risk: "none".into(),
            payload: serde_json::json!({ "text": "" }),
        };
        let c = chip(1000);
        let outcome = execute_action(&action, &c).await;
        assert!(matches!(outcome, Outcome::Refused { .. }));
    }
}
