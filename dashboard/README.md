# Faz 6 — Thornhill Ambient Web Console

Yerel tarayıcıdan `http://127.0.0.1:9099` üzerinden açılan Görsel Zihin Paneli.
**Harici bağımlılık yok**: HTTP/SSE elle yazılmıştır (tokio üstünde, `src/http/mod.rs`),
frontend saf HTML/CSS/Vanilla JS, D3 v7.9.0 vendor edilmiş tek dosyadır.

## ADR-062 Uyumu (statik dosya çözümleme sırası)

`GET /style.css`, `/app.js`, `/d3.v7.min.js`, `index.html` istekleri sırayla:

1. **XDG override dizini:** `$XDG_DATA_HOME/thornhill/dashboard/<dosya>`
   (varsa öncelikli sunulur — kullanıcı düzenlemesi binary'i yeniden derlemeden geçer)
2. **Binary'e gömülü:** `include_bytes!("../../dashboard/<dosya>")` —
   `thornhilld` tek ikilisi her ortamda konsolu taşır (Nix sandbox uyumlu)

Yol adayı `sanitize_rel_path()` ile temizlenir: `..`, mutlak yol, ters bölü,
NUL ve gizli dosyalar (`.` öneki) reddedilir (birim testli).

## Rotalar

| Yöntem | Yol | Koruma | İşlev |
|---|---|---|---|
| GET | `/` , `/index.html`, `/style.css`, `/app.js`, `/d3.v7.min.js` | Host kontrolü | statik panel |
| GET | `/api/stream` | Host + Token | SSE: `event.push` + `chip.push` canlı akış |
| GET | `/api/stats` | Host + Token | graf telemetrisi |
| GET | `/api/query?q=recent\|causal\|lessons&limit=N` | Host + Token | yapılandırılmış graf sorgusu |
| POST | `/api/ask` | Host + Origin + Token | S2 sorusu |
| POST | `/api/forget` | Host + Origin + **token** | §8.6 unutma hakkı |
| POST | `/api/chip-act` | Host + Origin + **token** | §7.2/§8.5 çip eylemi |

## Güvenlik sınırları (RFC-001 §8 karşılıkları)

- **Varsayılan bağlama:** `127.0.0.1:9099` (`THORNHILL_HTTP_ADDR` ile değiştirilir).
- **Non-loopback yasak:** `THORNHILL_HTTP_ADDR` loopback dışıysa sunucu,
  `THORNHILL_HTTP_ALLOW_REMOTE=1` açık opt-in olmadıkça **başlamaz**.
- **CSRF/DNS-rebinding:** `Host` başlığı loopback değilse 403; mutasyonlarda
  `Origin` ya yok ya loopback olmalı.
- **Token:** açılışta `/dev/urandom`'dan 16 bayt hex üretilip
  `$XDG_RUNTIME_DIR/thornhill/http.token` (0600) dosyasına yazılır;
  Tüm sorgu, akış ve mutasyon isteklerinde `X-Thornhill-Token` başlığı veya `?token=` parametresi zorunludur.
- **Body sınırı:** 64 KB üstü 413.

## Canlı test

```bash
thornhilld --verbose &
# panel: tarayıcıda http://127.0.0.1:9099
thornhill-ctl inject --src shell --kind cmd.done --subject test \
  --ctx '{"exit_code": 101, "cmd": "cargo build", "error": "E0432"}'
# Beklenen: Ticker'da kırmızı cmd.done, sağda Action Chip kartı,
# grafte cmd.done düğümü; ask konsolundan S2 yanıtı (Ollama çalışıyorsa).
```

## Yapı

```
src/http/mod.rs        # minimal HTTP/1.1 + SSE sunucusu + güvenlik fonksiyonları (4 test)
dashboard/index.html   # HUD + panel yerleşimi
dashboard/style.css    # Enyxma paleti (#121417/#1e2229/#e06c75/#61afef/#98c379/#e0af68)
dashboard/app.js       # D3 force graph, G3 gecikme grafiği, SSE ticker, ask, forget, chips
dashboard/d3.v7.min.js # D3 v7.9.0 (vendor: jsDelivr npm/d3@7, 274 KB)
```
