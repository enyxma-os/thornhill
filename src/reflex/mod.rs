//! RFC-001 §9.1 — Sistem 1 (Refleks Çekirdeği).
//!
//! Katı kurallar (RFC-001 §9.1):
//! - LLM çağrısı YOKTUR; VRAM kullanımı 0.
//! - Yanıt bütçesi: p99 < 1 ms (G3). `evaluate()` bunu kendi içinde ölçer.
//! - Kurallar deterministik: (src, kind) + ctx predicate -> Action Chip (§7.2).
//!
//! Faz 5 ekleri:
//! - `is_escalation_candidate`: §9.3.1 (yeni desen) yükseltme adayı testi.

use crate::protocol::wire::{ActionChip, ChipAction};
use std::time::{Duration, Instant};
use tracing::debug;

pub struct ReflexEngine;

fn make_chip(trigger: String, title: String, why: String, actions: Vec<ChipAction>) -> ActionChip {
    let labels = actions.iter().map(|a| a.label.clone()).collect();
    ActionChip {
        id: format!("chip_{}", ulid::Ulid::new()),
        trigger,
        title,
        why,
        labels,
        actions,
        ttl_ms: 60_000,
        conf: 1.0,
    }
}

fn copy_action(label: &str, text: &str) -> ChipAction {
    ChipAction {
        label: label.to_string(),
        action_type: "copy".to_string(),
        risk: "none".to_string(),
        payload: serde_json::json!({ "text": text }),
    }
}

fn exec_action(label: &str, risk: &str, cmd: &str) -> ChipAction {
    ChipAction {
        label: label.to_string(),
        action_type: "exec".to_string(),
        risk: risk.to_string(),
        payload: serde_json::json!({ "cmd": cmd }),
    }
}

impl ReflexEngine {
    pub fn new() -> Self {
        Self
    }

    /// S1 değerlendirme girişi. Bütçeyi ölçer, aşarsa uyarır.
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
            ("shell", "cmd.done") => {
                let exit_code = ev.ctx.get("exit_code")?.as_i64()?;
                if exit_code == 0 {
                    return None;
                }
                let cmd = ev
                    .ctx
                    .get("cmd")
                    .and_then(|v| v.as_str())
                    .unwrap_or("bilinmeyen komut")
                    .trim();
                let err = ev.ctx.get("error").and_then(|v| v.as_str()).unwrap_or("");
                let trigger = format!("cmd.done{{exit={}, cmd~'{}'}}", exit_code, cmd);

                if exit_code == 127 {
                    let bin = cmd.split_whitespace().next().unwrap_or(cmd);
                    return Some(make_chip(
                        trigger,
                        format!("Komut bulunamadı: {}", bin),
                        "PATH içinde bulunamadı — Nix paketlerinde arayabilir veya komutu kopyalayabilirsiniz"
                            .to_string(),
                        vec![
                            exec_action(
                                "Nix Paket Ara",
                                "none",
                                &format!("nix search nixpkgs#{} 2>&1 | head -n 20", bin),
                            ),
                            copy_action("Komutu Kopyala", cmd),
                        ],
                    ));
                }

                if exit_code == 126 {
                    return Some(make_chip(
                        trigger,
                        format!("İzin reddedildi: {}", cmd),
                        "Yürütme izni eksik — sudo ile kopyalayıp deneyebilirsiniz".to_string(),
                        vec![
                            copy_action("Sudo Komutunu Kopyala", &format!("sudo {}", cmd)),
                            exec_action("Komutu Yeniden Çalıştır", "medium", cmd),
                        ],
                    ));
                }

                if cmd.starts_with("git ") || cmd == "git" {
                    return Some(make_chip(
                        trigger,
                        format!("Git komutu başarısız: {}", cmd),
                        if err.is_empty() {
                            format!(
                                "Çıkış kodu {} — depo durumunu inceleyebilirsiniz",
                                exit_code
                            )
                        } else {
                            format!("Hata çıktısı: {}", err)
                        },
                        vec![
                            exec_action("Git Durumu", "none", "git status -sb"),
                            exec_action("Komutu Yeniden Çalıştır", "medium", cmd),
                        ],
                    ));
                }

                if cmd.starts_with("systemctl ") || cmd == "systemctl" {
                    return Some(make_chip(
                        trigger,
                        format!("Servis komutu başarısız: {}", cmd),
                        if err.is_empty() {
                            format!(
                                "Çıkış kodu {} — başarısız kullanıcı servislerini listeleyebilirsiniz",
                                exit_code
                            )
                        } else {
                            format!("Hata çıktısı: {}", err)
                        },
                        vec![
                            exec_action(
                                "Başarısız Servisler",
                                "none",
                                "systemctl --user --failed --no-pager",
                            ),
                            exec_action("Komutu Yeniden Çalıştır", "medium", cmd),
                        ],
                    ));
                }

                if cmd.starts_with("enyxma ") || cmd.starts_with("nixos-rebuild ") {
                    return Some(make_chip(
                        trigger,
                        format!("Sistem komutu başarısız: {}", cmd),
                        if err.is_empty() {
                            format!(
                                "Çıkış kodu {} — komutu kopyalayın veya tekrar deneyin",
                                exit_code
                            )
                        } else {
                            format!("Hata çıktısı: {}", err)
                        },
                        vec![
                            copy_action("Komutu Kopyala", cmd),
                            exec_action("Komutu Yeniden Çalıştır", "medium", cmd),
                        ],
                    ));
                }

                Some(make_chip(
                    trigger,
                    format!("Komut hatası: {} · çıkış {}", cmd, exit_code),
                    if err.is_empty() {
                        "S1 refleks kuralı: sıfırdan farklı çıkış kodu".to_string()
                    } else {
                        format!("Hata çıktısı: {}", err)
                    },
                    vec![
                        copy_action("Komutu Kopyala", cmd),
                        exec_action("Komutu Yeniden Çalıştır", "medium", cmd),
                        ChipAction {
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
                    ],
                ))
            }
            _ => None,
        }
    }
}

impl Default for ReflexEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// §9.3.1 (Faz 5): S1'in kaçırdığı **sistemik** olaylar S2 yükseltme adayıdır.
/// Saf fonksiyon — kararı bus kaydeder (`s1_escalate` olayı, §9.3 karar günlüğü).
pub fn is_escalation_candidate(ev: &crate::protocol::CanonicalEvent, s1_fired: bool) -> bool {
    if s1_fired {
        return false; // §9.3: S1 çözdüyse yükseltme yok
    }
    let systemic_failure = matches!((ev.src.as_str(), ev.kind.as_str()), ("shell", "cmd.done"))
        && ev
            .ctx
            .get("exit_code")
            .and_then(|v| v.as_i64())
            .map(|c| c != 0)
            .unwrap_or(false);
    systemic_failure
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
        assert_eq!(chip.labels.len(), 3);
        assert_eq!(chip.labels[0], "Komutu Kopyala");
        assert_eq!(chip.actions[0].action_type, "copy");
        assert_eq!(chip.actions[1].risk, "medium");
        assert_eq!(chip.actions[2].action_type, "ask_llm");
        assert_eq!(chip.conf, 1.0);
    }

    #[test]
    fn fires_on_command_not_found() {
        let engine = ReflexEngine::new();
        let chip = engine
            .evaluate(&shell_event(127, "htop --tree", ""))
            .expect("127 çip üretmeli");
        assert!(chip.title.contains("htop"));
        assert_eq!(chip.labels, vec!["Nix Paket Ara", "Komutu Kopyala"]);
        assert_eq!(chip.actions[0].action_type, "exec");
        assert_eq!(chip.actions[0].risk, "none");
        assert_eq!(chip.actions[1].action_type, "copy");
    }

    #[test]
    fn fires_on_git_and_systemctl_failures() {
        let engine = ReflexEngine::new();
        let git_chip = engine
            .evaluate(&shell_event(1, "git push origin main", ""))
            .expect("git çip üretmeli");
        assert_eq!(git_chip.labels[0], "Git Durumu");

        let sys_chip = engine
            .evaluate(&shell_event(3, "systemctl --user status foo", ""))
            .expect("systemctl çip üretmeli");
        assert_eq!(sys_chip.labels[0], "Başarısız Servisler");
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
        let engine = ReflexEngine::new();
        let ev = CanonicalEvent::new("shell", "cmd.done", "1", json!({}));
        assert!(engine.evaluate(&ev).is_none());
    }

    #[test]
    fn escalation_candidate_only_when_s1_misses() {
        // S1 çözdüyse asla yükselmez
        assert!(!is_escalation_candidate(&shell_event(1, "x", ""), true));
        // S1 kaçırdı + sistemik hata → aday
        assert!(is_escalation_candidate(&shell_event(1, "x", ""), false));
        // Başarılı komut asla aday değil
        assert!(!is_escalation_candidate(&shell_event(0, "x", ""), false));
        // Sistemik olmayan kaynaklar aday değil
        let ev = CanonicalEvent::new("hyprland", "win.focus", "kitty", json!({}));
        assert!(!is_escalation_candidate(&ev, false));
    }
}
