//! RFC-001 §6: Nedensellik Grafiği (SQLite, WAL).
//!
//! Tüm erişim daemon içindendir; dış süreçler yalnızca UDS sorgu mesajlarıyla
//! okur (RFC-001 §6.1). Bu dosya §6.2'deki DDL'i birebir uygular.

use crate::protocol::CanonicalEvent;
use rusqlite::{params, Connection};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

/// RFC-001 §6.2 — birebir DDL (PRAGMA dahil).
const DDL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;

CREATE TABLE IF NOT EXISTS events (
  id        TEXT PRIMARY KEY,
  ts        INTEGER NOT NULL,
  src       TEXT    NOT NULL,
  kind      TEXT    NOT NULL,
  subject   TEXT    NOT NULL,
  ctx       TEXT    NOT NULL DEFAULT '{}',
  conf      REAL    NOT NULL DEFAULT 1.0,
  ref       TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_ts   ON events(ts);
CREATE INDEX IF NOT EXISTS idx_events_kind ON events(kind, ts);
CREATE INDEX IF NOT EXISTS idx_events_subj ON events(subject, ts);

CREATE TABLE IF NOT EXISTS edges (
  id        INTEGER PRIMARY KEY,
  src_ev    TEXT NOT NULL REFERENCES events(id),
  dst_ev    TEXT NOT NULL REFERENCES events(id),
  rel       TEXT NOT NULL,
  conf      REAL NOT NULL DEFAULT 0.5,
  support   INTEGER NOT NULL DEFAULT 1,
  updated   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src_ev, rel);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst_ev, rel);
CREATE UNIQUE INDEX IF NOT EXISTS uq_edges ON edges(src_ev, dst_ev, rel);

CREATE TABLE IF NOT EXISTS lessons (
  id        INTEGER PRIMARY KEY,
  pattern   TEXT NOT NULL,
  summary   TEXT NOT NULL,
  remedy    TEXT,
  score     REAL NOT NULL DEFAULT 0.5,
  uses      INTEGER NOT NULL DEFAULT 0,
  last_used INTEGER,
  created   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS habits (
  id          INTEGER PRIMARY KEY,
  context_sig TEXT NOT NULL,
  action_sig  TEXT NOT NULL,
  freq        INTEGER NOT NULL DEFAULT 1,
  last        INTEGER NOT NULL,
  UNIQUE(context_sig, action_sig)
);
"#;

/// Graf deposu. `rusqlite::Connection` `Sync` olmadığı için `std::sync::Mutex`
/// ile korunur; kritik bölgeler mikrosaniye düzeyinde brief işlemlerdir ve
/// asla `.await` noktası boyunca tutulmaz.
pub struct GraphStore {
    conn: Arc<Mutex<Connection>>,
}

/// RFC-001 §6.1: `$XDG_STATE_HOME/thornhill/graph.db` (ADR-062: sabit yol yok).
pub fn resolve_db_path() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    base.join("thornhill").join("graph.db")
}

impl GraphStore {
    /// Depoyu açar, WAL moduna alır ve DDL'i uygular (idempotent).
    pub fn open(path: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db_path = path.unwrap_or_else(resolve_db_path);

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
            let perms = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(parent, perms)?;
        }

        let conn = Connection::open(&db_path)?;
        conn.execute_batch(DDL)?;

        let journal: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        info!("👁️ Grafi açıldı: {:?} (journal_mode={})", db_path, journal);

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Kanonik olayı kalıcılaştırır (RFC-001 §5 şeması, tek gerçek kaynak).
    pub fn append(
        &self,
        ev: &CanonicalEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.conn.lock().expect("graph mutex zehirlendi");
        let ctx = serde_json::to_string(&ev.ctx)?;
        conn.execute(
            "INSERT INTO events (id, ts, src, kind, subject, ctx, conf, ref)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ev.id,
                ev.ts as i64,
                ev.src,
                ev.kind,
                ev.subject,
                ctx,
                ev.conf,
                ev.ref_id
            ],
        )?;
        Ok(())
    }

    /// RFC-001 §7.1 `query` op'u: yapılandırılmış, salt-okunur sorgu.
    /// Ham SQL tel üzerinden ASLA kabul edilmez (§8 güvenlik modeli).
    pub fn query(
        &self,
        q: &str,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.conn.lock().expect("graph mutex zehirlendi");
        let limit = limit.clamp(1, 1000);

        match q {
            // Son olaylar (en yeni önce)
            "recent" => {
                let mut stmt = conn.prepare(
                    "SELECT id, ts, src, kind, subject, ctx, conf
                     FROM events ORDER BY ts DESC LIMIT ?1",
                )?;
                let rows = stmt
                    .query_map(params![limit], Self::row_to_json)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            // Nedensellik zincirleri: kenarlar + iki uç olayın özeti
            "causal" => {
                let mut stmt = conn.prepare(
                    "SELECT e.src_ev, se.kind, e.rel, e.conf, e.support, e.dst_ev, de.kind
                     FROM edges e
                     JOIN events se ON se.id = e.src_ev
                     JOIN events de ON de.id = e.dst_ev
                     ORDER BY e.conf DESC LIMIT ?1",
                )?;
                let rows = stmt
                    .query_map(params![limit], |row| {
                        Ok(serde_json::json!({
                            "src_ev": row.get::<_, String>(0)?,
                            "src_kind": row.get::<_, String>(1)?,
                            "rel": row.get::<_, String>(2)?,
                            "conf": row.get::<_, f64>(3)?,
                            "support": row.get::<_, i64>(4)?,
                            "dst_ev": row.get::<_, String>(5)?,
                            "dst_kind": row.get::<_, String>(6)?,
                        }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            other => Err(format!(
                "Bilinmeyen sorgu türü: {:?} (geçerli: recent, causal)",
                other
            )
            .into()),
        }
    }

    /// Telemetri: tablo satır sayıları.
    pub fn stats(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.conn.lock().expect("graph mutex zehirlendi");
        let count = |name: &str| -> rusqlite::Result<i64> {
            conn.query_row(&format!("SELECT COUNT(*) FROM {}", name), [], |r| r.get(0))
        };
        Ok(serde_json::json!({
            "events": count("events")?,
            "edges": count("edges")?,
            "lessons": count("lessons")?,
            "habits": count("habits")?,
        }))
    }

    fn row_to_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
        let ctx_raw: String = row.get(5)?;
        let ctx_val: serde_json::Value =
            serde_json::from_str(&ctx_raw).unwrap_or(serde_json::Value::Object(Default::default()));
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "ts": row.get::<_, i64>(1)?,
            "src": row.get::<_, String>(2)?,
            "kind": row.get::<_, String>(3)?,
            "subject": row.get::<_, String>(4)?,
            "ctx": ctx_val,
            "conf": row.get::<_, f64>(6)?,
        }))
    }
}

// §4.2 drop_policy: broadcast Lagged'ta kaçıran olay UDS ile zaten gelmiş
// ve append() ile kalıcılaşmıştır; canlı abonenin kaçırması yalnızca akış
// kaybıdır — uyarı loglanır, daemon düşmez.
#[allow(dead_code)]
fn _lag_note() {
    warn!("placeholder: belge amaçlı");
}
