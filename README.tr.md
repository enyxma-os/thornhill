# Enyxma Thornhill

> **Dil / Language:** [English](./README.md) | **Türkçe**

Linux masaüstünde sistem olaylarını izleyen, olaylar arasında neden-sonuç bağı kuran ve bu bağlardan türeyen eylem kartlarını kullanıcı onayına sunan, Rust ile yazılmış yerel servis.

- Depo: https://codeberg.org/enyxma/thornhill
- Lisans: GPL-3.0-or-later — [LICENSE](./LICENSE)
- Dil: Rust, asenkron tokio çalışma zamanı
- Üretilen ikililer: `thornhilld` (servis) ve `thornhill-ctl` (komut satırı aracı)
- Paketleme: Nix Flake; `packages.default`, `nixosModules.default` ve `homeManagerModules.default` çıktıları

## Mimari

```
+------------------------------+      +---------------------------------+
| Duyusal Hat                  |      | Nedensellik Grafiği             |
| Hyprland socket2 olay akışı  |----->| SQLite WAL                      |
| kabuk kancaları: fish, zsh,  |olay  | tablolar: events, edges,        |
| bash                         |      | lessons, habits                 |
+------------------------------+      +----------------+----------------+
                                                |
                                 +--------------v---------------+
                                 | S1 Refleks Motoru            |
                                 | deterministik kurallar       |
                                 | hedef: 1 ms altı, 0 VRAM     |
                                 +--------------+---------------+
                                                | Action Chip
+------------------------------+      +--------v-----------------------+
| S2 Bilişsel Katman           |      | Eylem Yürütücü                 |
| yerel Ollama sorgusu         |<-----| onay merdiveni ve TTL kayıt    |
| rate limit, pil kilidi       |      | defteri                        |
+------------------------------+      +--------------------------------+
       UNIX Domain Socket: $XDG_RUNTIME_DIR/thornhill/thornhill.sock (NDJSON)
+------------------------------+      +---------------------------------+
| Quickshell QML katmanı       |      | Web konsolu                     |
| shell/ dizini                |      | http://127.0.0.1:9099           |
+------------------------------+      +---------------------------------+
```

### Duyusal Hat

- Hyprland'ın `$XDG_RUNTIME_DIR/hypr/<HIS>/.socket2.sock` olay akışını okur. Oturum keşfi `HYPRLAND_INSTANCE_SIGNATURE` ortam değişkeninden yapılır; değişken yoksa `hypr/` altında `.socket2.sock` içeren dizin taranır. Sabit yol kodda bulunmaz.
- Kabuk kancaları `shell-hook/thornhill.sh` ve `shell-hook/thornhill.fish` dosyalarındadır. Yalnızca çıkış kodu sıfırdan farklı olan komutlar raporlanır; araç veya soket yoksa kanca sessizce sonlanır.
- Tüm dinleme olay güdümlüdür. Bağlantı kopması üstel geri çekilmeyle yeniden bağlanır; periyodik sorgu döngüsü yoktur.

### Nedensellik Grafiği

- Veritabanı yolu: `$XDG_STATE_HOME/thornhill/graph.db`, WAL modu.
- `events` tablosu kanonik olayları, `edges` tablosu `caused` gibi ilişki türleriyle destek ve güven puanlarını tutar. Aynı desen üç kez görüldüğünde kayıt `lessons` tablosuna kalıcı özet olarak yükselir.
- `forget` komutu verilen tarihten eski olayları ve bağlı kenarları siler; `lessons` ve `habits` korunur.

### S1 Refleks Motoru

- Dil modeli çağırmaz, VRAM kullanmaz. Başarısız komut gibi tanımlı durumlarda deterministik kuralla Action Chip üretir; hedef yanıt süresi 1 ms altıdır.
- Action Chip, etiketler ve risk sınıfı taşıyan eylemlerden oluşur.

### S2 Bilişsel Katman

- Yalnızca kullanıcı doğrudan sorduğunda veya çip üzerindeki açıklama eylemi seçildiğinde devreye girer.
- Grafin son olayları, kenarları ve öğrenilmiş derslerden oluşan bağlam paketini yerel Ollama API'sine tek istekte gönderir; yanıt `explanation` ve `remedy` alanlarından oluşan JSON şemasına zorlanır.
- Rate limit: kullanıcı sorusu için 2 saniye, çip kaynaklı çağrı için 10 saniye; damga yalnız başarılı çağrı sonrası yazılır.
- Sistem pil ile çalışırken S2 varsayılan olarak kilitlenir.

### Masaüstü ve Web Arayüzleri

- `shell/` dizinindeki QML bileşenleri Quickshell üzerinden çip kartlarını masaüstünde gösterir.
- Web konsolu daemon içinde çalışan gömülü HTTP ve SSE sunucusudur; harici web sunucusu gerektirmez. Varsayılan adres `http://127.0.0.1:9099`, adres `THORNHILL_HTTP_ADDR` ile değiştirilir.
- Panel bölümleri: D3.js force-directed nedensellik grafiği, olay akış aralığı grafiği, canlı olay akışı, Action Chip listesi, S2 sorgu konsolu ve unutma hakkı düğmeleri.
- Statik dosyalar `$XDG_DATA_HOME/thornhill/dashboard` dizininden yüklenir; dizin yoksa ikili içine gömülü kopya kullanılır.

## Hızlı Başlangıç

Kaynak koddan derleme, test ve çalıştırma:

```bash
git clone https://codeberg.org/enyxma/thornhill.git
cd thornhill
nix develop -c cargo test
nix develop -c just verify
nix build .#packages.x86_64-linux.default --no-link
./result/bin/thornhilld --verbose
```

Servis olmadan elle çalıştırma sırasında daemon UDS soketini ve web konsolunu açar. Sonlandırma SIGINT veya SIGTERM ile yapılır.

## Komut Satırı Kullanımı

```bash
# daemon durumu ve özellik listesi
thornhill-ctl status

# graf telemetrisi: olay, kenar, ders ve alışkanlık sayaçları
thornhill-ctl stats

# sentetik olay enjekte etme; refleks motorunu tetikler
thornhill-ctl inject --src shell --kind cmd.done --subject test \
  --ctx '{"exit_code": 101, "cmd": "cargo build", "error": "E0432"}'

# canlı olay ve çip akışı
thornhill-ctl listen

# yapılandırılmış graf sorgusu: recent, causal, lessons
thornhill-ctl query -q causal

# S2 bilişsel katmana soru sorma
thornhill-ctl ask "son derleme hatasının nedenini açıkla"

# unutma hakkı: varsayılan 30 günden eski olayları siler
thornhill-ctl forget --before 7d

# çip eylemi: önce önizleme, ardından onay
thornhill-ctl chip-act chip_01M5DMB2M88REZC5R6F181SBY1
thornhill-ctl chip-act chip_01M5DMB2M88REZC5R6F181SBY1 --confirm
```

## Web Konsolu

Adres: `http://127.0.0.1:9099`. Bağlama adresi `THORNHILL_HTTP_ADDR` ile değiştirilir; loopback dışı adres yalnız `THORNHILL_HTTP_ALLOW_REMOTE=1` ile kabul edilir.

| Yol | Yöntem | İşlev |
|---|---|---|
| `/` | GET | panel |
| `/api/stream` | GET | SSE ile canlı olay ve çip akışı |
| `/api/stats` | GET | graf sayaçları |
| `/api/query` | GET | recent, causal, lessons sorguları |
| `/api/ask` | POST | S2 sorusu |
| `/api/forget` | POST | eski olayların silinmesi |
| `/api/chip-act` | POST | çip eylemi |

## Güvenlik ve Onay Merdiveni

- UDS soketi 0700 izinleriyle açılır ve `SO_PEERCRED` ile eş UID doğrulaması yapılır; farklı kullanıcıdan gelen bağlantı ilk mesajda kapatılır.
- Web konsolu yalnız loopback üzerinde çalışır. `Host` başlığı loopback değilse istek 403 ile reddedilir; mutasyon yolları `Origin` kontrolü ve `X-Thornhill-Token` başlığı ister. Token açılışta `$XDG_RUNTIME_DIR/thornhill/http.token` dosyasına 0600 izinleriyle yazılır.
- Eylem risk sınıfları:

| Risk | Davranış |
|---|---|
| none | tek tık ile yürütülür |
| low | dry-run doğrulaması çalışır; uygulama yapılmaz |
| medium | iki adımlı onay: önizleme, ardından basılı tutma jesti |
| high | çalıştırılmaz; yalnız öneri olarak sunulur |

- Statik dosya isteklerinde yol temizliği yapılır; `..`, mutlak yol ve gizli dosya adayları reddedilir.
- Veri varsayılan olarak makineden çıkmaz. S2 çağrısı yapılandırılmış endpoint'e gider; varsayılan endpoint yerel Ollama'dır.
