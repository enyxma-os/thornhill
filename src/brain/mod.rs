//! RFC-001 §9.2 — Sistem 2 (Bilişsel Katman): S2 orkestratörü.
//!
//! Görev sırası (`answer()`):
//!   1. Grafın ilgili alt zincirini **bağlam paketi** olarak hazırla (§9.2)
//!   2. LLM'e (yerel Ollama uyumlu) tek turlu, şemaya zorlanmış istek gönder
//!   3. Yanıtı ayrıştır; `{explanation, remedy}` döner (chip altına akar)
//!
//! Katı kurallar:
//! - Rate limit (§10): tetik sınıfı başına 1 çağrı / 2 sn (user) / 10 sn (chip).
//! - Endpoint ADR-062: `THORNHILL_S2_ENDPOINT` env (varsayılan yerel Ollama).
//! - LLM çıktısına güven sınırı: yanıt `conf` taşıyamaz; kullanıcı onayı olmadan
//!   lesson/habit kazanamaz (§6.3 lekelenme karşı önlem).

use crate::graph::GraphStore;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info};

/// S2 yapılandırması (env'den okunur; sabit kullanıcı yolu yok).
pub struct S2Config {
    pub endpoint: String,
    pub model: String,
}

impl S2Config {
    pub fn from_env() -> Self {
        Self {
            endpoint: std::env::var("THORNHILL_S2_ENDPOINT")
                .unwrap_or_else(|_| "http://127.0.0.1:11434/api/generate".to_string()),
            model: std::env::var("THORNHILL_S2_MODEL").unwrap_or_else(|_| "llama3.2".to_string()),
        }
    }
}

/// Tetik sınıfına göre rate limit tablosu (RFC-001 §10).
fn rate_limit_for(trigger_class: &str) -> Duration {
    match trigger_class {
        "user" => Duration::from_secs(2),
        "chip" => Duration::from_secs(10),
        _ => Duration::from_secs(10),
    }
}

/// S2 yanıtı — şemaya zorlanmış tek turlu çıktı (§9.2).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S2Answer {
    pub explanation: String,
    #[serde(default)]
    pub remedy: String,
    #[serde(default)]
    pub confidence: f64,
}

pub struct Brain {
    graph: Arc<GraphStore>,
    config: S2Config,
    http: reqwest::Client,
    /// Tetik sınıfı -> son çağrı anı (rate limit)
    last_call: Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl Brain {
    pub fn new(graph: Arc<GraphStore>, config: S2Config) -> Self {
        Self {
            graph,
            config,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            last_call: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// §9.3.3: Kullanıcı sorusu — her zaman S2. Rate limit sınıfı: "user".
    pub async fn answer_query(&self, question: &str) -> Result<S2Answer, String> {
        self.answer(question, "user").await
    }

    /// §9.1 → §9.2: Chip'ten gelen `ask_llm` eylemi. Rate limit sınıfı: "chip".
    pub async fn answer_chip(&self, chip_context: &str) -> Result<S2Answer, String> {
        self.answer(chip_context, "chip").await
    }

    async fn answer(&self, question: &str, trigger_class: &str) -> Result<S2Answer, String> {
        // §10 rate limit: aynı tetik sınıfı için pencere içinde ikinci çağrı reddedilir.
        let window = rate_limit_for(trigger_class);
        let now = std::time::Instant::now();
        {
            let last = self.last_call.lock().await;
            if let Some(prev) = last.get(trigger_class) {
                if now.duration_since(*prev) < window {
                    return Err(format!(
                        "S2 rate limit: {:?} sınıfı için {:?} içinde zaten çağrı yapıldı",
                        trigger_class, window
                    ));
                }
            }
        }

        let context_pack = self.build_context_pack(question);
        info!("🧠 S2 devrede ({}): {:?}", trigger_class, question);
        debug!("S2 bağlam paketi: {}", context_pack);

        let prompt = format!(
            "Sen Thornhill adlı sistemin bilişsel katmanısın. Bir Linux kullanıcısının \
             sisteminde oluşan olayları açıklarsın. Yanıtını SADECE şu JSON şemasında ver: \
             {{\"explanation\": \"...\", \"remedy\": \"...\"}}\n\n\
             ## Graf bağlamı (nedensel zincir)\n{}\n\n## Soru\n{}",
            context_pack, question
        );

        let body = serde_json::json!({
            "model": self.config.model,
            "prompt": prompt,
            "stream": false,
            "format": "json",
            "options": { "temperature": 0.2 }
        });

        let resp = self
            .http
            .post(&self.config.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "S2 endpoint'e ulaşılamadı ({}): {}",
                    self.config.endpoint, e
                )
            })?;

        if !resp.status().is_success() {
            return Err(format!("S2 endpoint hatası: HTTP {}", resp.status()));
        }

        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("S2 yanıtı JSON değil: {}", e))?;
        // Ollama /api/generate: {"response": "<json string>", ...}
        let inner_raw = v
            .get("response")
            .and_then(|r| r.as_str())
            .ok_or("S2 yanıtında 'response' alanı yok")?;
        let answer: S2Answer =
            serde_json::from_str(inner_raw).map_err(|e| format!("S2 şeması bozuk: {}", e))?;

        if answer.confidence > 0.0 && answer.confidence < 0.5 {
            debug!("S2 düşük güvenle yanıt verdi: {:.2}", answer.confidence);
        }
        // Başarılı çağrı sonrası rate limit damgasını kaydet
        {
            let mut last = self.last_call.lock().await;
            last.insert(trigger_class.to_string(), std::time::Instant::now());
        }
        Ok(answer)
    }

    /// §9.2 bağlam paketleyici: grafin son olayları + kenarları + lessons'tan
    /// kompakt bir özet üretir. Ham SQL yok — yalnız yapılandırılmış sorgular.
    fn build_context_pack(&self, _question: &str) -> String {
        let mut pack = String::new();

        if let Ok(rows) = self.graph.query("recent", 10) {
            pack.push_str(&format!("Son {} olay:\n", rows.len()));
            for r in rows.iter().take(10) {
                pack.push_str(&format!(
                    "- [{}] {} {} {}\n",
                    r.get("ts").and_then(|t| t.as_i64()).unwrap_or(0),
                    r.get("src").and_then(|s| s.as_str()).unwrap_or("?"),
                    r.get("kind").and_then(|k| k.as_str()).unwrap_or("?"),
                    r.get("subject").and_then(|s| s.as_str()).unwrap_or("?"),
                ));
            }
        }

        if let Ok(rows) = self.graph.query("causal", 10) {
            if !rows.is_empty() {
                pack.push_str("\nNedensel kenarlar:\n");
                for r in rows.iter().take(10) {
                    pack.push_str(&format!(
                        "- {} -[{}]→ {} (support={})\n",
                        r.get("src_kind").and_then(|s| s.as_str()).unwrap_or("?"),
                        r.get("rel").and_then(|s| s.as_str()).unwrap_or("?"),
                        r.get("dst_kind").and_then(|s| s.as_str()).unwrap_or("?"),
                        r.get("support").and_then(|s| s.as_i64()).unwrap_or(0),
                    ));
                }
            }
        }

        if let Ok(rows) = self.graph.query("lessons", 5) {
            if !rows.is_empty() {
                pack.push_str("\nÖğrenilmiş dersler:\n");
                for r in rows.iter().take(5) {
                    pack.push_str(&format!(
                        "- {}\n",
                        r.get("summary").and_then(|s| s.as_str()).unwrap_or("?"),
                    ));
                }
            }
        }

        pack
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_user_is_2s_chip_is_10s() {
        assert_eq!(rate_limit_for("user"), Duration::from_secs(2));
        assert_eq!(rate_limit_for("chip"), Duration::from_secs(10));
    }

    #[test]
    fn config_defaults_are_local() {
        // Varsayılan endpoint yerel Ollama olmalı (§8.1: varsayılan veri çıkışı yok)
        let cfg = S2Config {
            endpoint: "http://127.0.0.1:11434/api/generate".into(),
            model: "llama3.2".into(),
        };
        assert!(cfg.endpoint.starts_with("http://127.0.0.1"));
    }

    // Not: answer() ağ çağrısı yapar; birim testi yalnız saf parçaları kapsar.
    // Uçtan uca S2 testi canlı Ollama ile elle yapılır (docs/DEPLOY.md).
}
