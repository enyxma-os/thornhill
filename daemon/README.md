# daemon/ — thornhilld

Tek süreçli daemon (`thornhilld`), kullanıcı başına `systemd --user` servisi.

## Alt birimler

- `sensory/` — Duyu kaynakları (RFC-001 §4):
  - Hyprland socket2 (`$XDG_RUNTIME_DIR/hypr/<HIS>/.socket2.sock`) canlı olay akışı
  - inotify (seçici, **recursive değil** — watch bütçesi ≤4096)
  - netlink proc connector (`CN_PROC`) + 5 sn'lik procfs uzlaştırma taraması
  - udev monitörü (ses/ekran cihazları)
- `graph/` — SQLite WAL; DDL RFC-001 §6.2'de sabittir. Erişim yalnızca daemon içinden.
- `bus/` — UDS sunucusu: `$XDG_RUNTIME_DIR/thornhill/thornhill.sock` (0700, NDJSON, op listesi §7.1)

## Katı kurallar

- Duyu asla bloklamaz; kuyruk dolarsa `coalesce` düşürme politikası.
- S1 karar yolu p99 < 1 ms; LLM çağrisi içermez.
- S2 rate limit: 1 çağrı / 10 sn / tetik sınıfı.
