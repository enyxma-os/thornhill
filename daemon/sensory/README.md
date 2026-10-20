# sensory/ — Duyusal Algı Katmanı

Sıfır-yük olay toplama. Kaynak → kanonik olay şeması (RFC-001 §5) dönüşümü.

| Dosya (planlanan) | Kaynak | Mekanizma |
|---|---|---|
| `hyprland.rs` | Pencere/WS | socket2 olay akışı: `workspace`, `windowtitle`, `activewindow`, `openwindow`, `closewindow`, `fullscreen` |
| `inotify.rs` | Dosya sistemi | seçici izleme; `max_user_watches` bütçesi 4096; aşılırsa 2 sn polling fallback |
| `gitpulse.rs` | Git | repo kökü + `.git/HEAD` + `.git/index` izleri; `git status --porcelain` olay-tetiklemeli |
| `proc.rs` | Süreçler | netlink proc connector (FORK/EXEC/EXIT); kısa ömürlü kaçaklar için procfs uzlaştırma |
| `udev.rs` | Donanım | libudev monitörü (ses çıkışı, kulaklık, ekran) |

## Filtre kuralları

- Kaynakta filtrele; daemon'a gereksiz bayt gelmez.
- Coalesce: aynı `kind`+`subject` ≤500 ms içinde n kez → tek olay, `ctx.count=n`.
- Gizli desen filtresi: `token|secret|password|key|credential` → olay düşürülür.
