# bus/ — UDS Tel Protokolü

`$XDG_RUNTIME_DIR/thornhill/thornhill.sock` — 0700, `SO_PEERCRED` eş UID doğrulaması.
Satır sınırlı NDJSON; zarf: `{"op":..., "req":"<ulid>", ...}`.

Ops: `hello`, `sub`, `unsub`, `event`, `query`, `chip.push`, `chip.act`, `feedback`.

## Action Chip

`chip.push` → Shell'de 1–3 eylemli çip (örn. `[Hatayı Açıkla]` `[Düzeltme Diff'ini Uygula]`).
`ttl_ms` dolarsa otomatik dismiss olur ve bu grafe kaydedilir.

**Kural:** `chip.act{confirm:true}` olmadan hiçbir `apply_patch`/`exec`/`config-write` çalışmaz.
Risk merdiveni: `none` (okuma) → `low` (diff, dry-run zorunlu) → `medium` (komut, hold-to-confirm) → `high` (v1'de yasak).
