//! RFC-001 §6: Nedensellik Grafiği (SQLite, WAL).
//!
//! Tüm erişim daemon içindendir; dış süreçler yalnızca UDS sorgu mesajlarıyla
//! okur (RFC-001 §6.1). Bu dosya §6.2'deki DDL'i birebir uygular.
//! Faz 2 Adım 2.2: link()/ensure_lesson() ile §6.3 destek mekanizması eklendi.

use crate::protocol::CanonicalEvent;
use rusqlite::{params, Connection};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::info;

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

/// Öğrenme eşiği: RFC-001 §6.3 — support >= 3 olan kenar lesson'a yükselir.
pub const LESSON_SUPPORT_THRESHOLD: i64 = 3;

/// Graf deposu. `rusqlite::Connection` `Sync` olmadığı için `std::sync::Mutex`
/// ile korunur; kritik bölgeler kısa işlemlerdir ve `.await` boyunca tutulmaz.
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

type GraphResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl GraphStore {
    /// Depoyu açar, WAL moduna alır ve DDL'i uygular (idempotent).
    pub fn open(path: Option<PathBuf>) -> GraphResult<Self> {
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
    pub fn append(&self, ev: &CanonicalEvent) -> GraphResult<()> {
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

    /// §6.3: İki olay arasına kenar kurar veya mevcut kenarın desteğini artırır.
    /// Yeni destek sayısını döner.
    pub fn link(&self, src_ev: &str, dst_ev: &str, rel: &str, conf: f64) -> GraphResult<i64> {
        let conn = self.conn.lock().expect("graph mutex zehirlendi");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO edges (src_ev, dst_ev, rel, conf, support, updated)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(src_ev, dst_ev, rel)
             DO UPDATE SET support = support + 1, conf = MAX(conf, ?4), updated = ?5",
            params![src_ev, dst_ev, rel, conf, now],
        )?;
        let support: i64 = conn.query_row(
            "SELECT support FROM edges WHERE src_ev = ?1 AND dst_ev = ?2 AND rel = ?3",
            params![src_ev, dst_ev, rel],
            |r| r.get(0),
        )?;
        Ok(support)
    }

    /// §6.3: Son `window_ms` içindeki olayları eskiden yeniye sıralı döner.
    /// Korelasyon penceresi (örn. git.dirty → build hatası) için kullanılır.
    pub fn recent_within(&self, window_ms: u64) -> GraphResult<Vec<(String, String, String)>> {
        let conn = self.conn.lock().expect("graph mutex zehirlendi");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut stmt = conn.prepare(
            "SELECT id, kind, subject FROM events
             WHERE ts >= ?1 AND ts <= ?2 ORDER BY ts ASC",
        )?;
        let rows = stmt
            .query_map(params![now - window_ms as i64, now], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// §6.3: Aynı desenden lesson yoksa oluşturur (idempotent).
    pub fn ensure_lesson(
        &self,
        pattern: &str,
        summary: &str,
        remedy: Option<&str>,
    ) -> GraphResult<bool> {
        let conn = self.conn.lock().expect("graph mutex zehirlendi");
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM lessons WHERE pattern = ?1)",
            params![pattern],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(false);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO lessons (pattern, summary, remedy, created) VALUES (?1, ?2, ?3, ?4)",
            params![pattern, summary, remedy, now],
        )?;
        Ok(true)
    }

    /// RFC-001 §7.1 `query` op'u: yapılandırılmış, salt-okunur sorgu.
    pub fn query(&self, q: &str, limit: u32) -> GraphResult<Vec<serde_json::Value>> {
        let conn = self.conn.lock().expect("graph mutex zehirlendi");
        let limit = limit.clamp(1, 1000);

        match q {
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
            "causal" => {
                let mut stmt = conn.prepare(
                    "SELECT e.src_ev, se.kind, e.rel, e.conf, e.support, e.dst_ev, de.kind
                     FROM edges e
                     JOIN events se ON se.id = e.src_ev
                     JOIN events de ON de.id = e.dst_ev
                     ORDER BY e.support DESC, e.conf DESC LIMIT ?1",
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
            "lessons" => {
                let mut stmt = conn.prepare(
                    "SELECT id, pattern, summary, remedy, score, uses FROM lessons
                     ORDER BY score DESC LIMIT ?1",
                )?;
                let rows = stmt
                    .query_map(params![limit], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, i64>(0)?,
                            "pattern": row.get::<_, String>(1)?,
                            "summary": row.get::<_, String>(2)?,
                            "remedy": row.get::<_, Option<String>>(3)?,
                            "score": row.get::<_, f64>(4)?,
                            "uses": row.get::<_, i64>(5)?,
                        }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            other => Err(format!(
                "Bilinmeyen sorgu türü: {:?} (geçerli: recent, causal, lessons)",
                other
            )
            .into()),
        }
    }

    /// Telemetri: tablo satır sayıları.
    pub fn stats(&self) -> GraphResult<serde_json::Value> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(src: &str, kind: &str, subject: &str) -> CanonicalEvent {
        CanonicalEvent::new(src, kind, subject, json!({}))
    }

    fn temp_store() -> GraphStore {
        GraphStore::open(Some(
            std::env::temp_dir().join(format!("thornhill-test-{}.db", ulid::Ulid::new())),
        ))
        .expect("test grafi açılamadı")
    }

    #[test]
    fn link_accumulates_support() {
        let g = temp_store();
        let a = ev("git", "git.dirty", "repo");
        let b = ev("shell", "cmd.done", "123");
        g.append(&a).unwrap();
        g.append(&b).unwrap();

        let s1 = g.link(&a.id, &b.id, "caused", 0.5).unwrap();
        let s2 = g.link(&a.id, &b.id, "caused", 0.5).unwrap();
        let s3 = g.link(&a.id, &b.id, "caused", 0.6).unwrap();
        assert_eq!((s1, s2, s3), (1, 2, 3));
        assert_eq!(s3, LESSON_SUPPORT_THRESHOLD);
    }

    #[test]
    fn ensure_lesson_is_idempotent() {
        let g = temp_store();
        assert!(g
            .ensure_lesson("build-fail", "derleme patlaması", Some("flag"))
            .unwrap());
        assert!(!g
            .ensure_lesson("build-fail", "derleme patlaması", Some("flag"))
            .unwrap());
    }

    #[test]
    fn recent_within_scopes_window() {
        let g = temp_store();
        g.append(&ev("git", "git.dirty", "repo")).unwrap();
        let rows = g.recent_within(10_000).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "git.dirty");
    }

    #[test]
    fn lessons_query_returns_rows() {
        let g = temp_store();
        g.ensure_lesson("p", "s", None).unwrap();
        let rows = g.query("lessons", 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["pattern"], "p");
    }
}
