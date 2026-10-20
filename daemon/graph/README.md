# graph/ — Nedensellik Grafiği

SQLite (WAL, `synchronous=NORMAL`), `$XDG_STATE_HOME/thornhill/graph.db`.

Tablolar (tam DDL RFC-001 §6.2'de): `events`, `edges`, `lessons`, `habits`.

## Çıkarım ilkesi

- S1 deterministik kurallarla kenar yazar (`caused`, `preceded`, `resolved_by`, `prefers`, `correlates`).
- `support ≥ 3` → lesson özetine yükselir.
- Güven sönümlenir (half-life 30 gün); kenarlar silinmez.
- S2 çıkarımları `conf < 0.5` ile girer; kullanıcı onayı olmadan support kazanamaz.
- Retention: events 30 gün, edges 180 gün (support≥3 kalıcı), lessons/habits süresiz.
