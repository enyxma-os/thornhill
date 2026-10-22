# Thornhill Dağıtım Kılavuzu

Bu kılavuz, Thornhill'i flake tabanlı bir NixOS yapılandırmasına ve Home Manager ile yönetilen kullanıcı ortamına dahil etmenin tek ve standart yolunu anlatır.

## 1. Flake Inputs Yapılandırması

NixOS yapılandırma flake'inizin girdilerine Thornhill eklenir:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    thornhill.url = "git+https://codeberg.org/enyxma/thornhill";
  };
}
```

Home Manager ayrı flake olarak yönetiliyorsa aynı girdi Home Manager flake'ine de eklenir ve `nixpkgs.follows` ile ortak nixpkgs kullanılır.

## 2. Home Manager Modülü ile Entegrasyon

Kullanıcı düzeyi servis ve kabuk kancaları için önerilen yol Home Manager modülüdür:

```nix
{ inputs, pkgs, ... }:

{
  imports = [ inputs.thornhill.homeManagerModules.default ];

  services.thornhill = {
    enable = true;
    enableShellHook = true;
  };

  home.packages = [
    inputs.thornhill.packages.${pkgs.system}.default
  ];
}
```

Seçenekler:

| Seçenek | Tür | Varsayılan | İşlev |
|---|---|---|---|
| `services.thornhill.enable` | boole | `false` | `thornhilld` kullanıcı servisini tanımlar |
| `services.thornhill.package` | paket | flake çıktısı | kullanılacak Thornhill paketi |
| `services.thornhill.enableShellHook` | boole | `false` | hata kancalarını kabuk başlangıcına ekler |

`enableShellHook` seçeneği etkinleştirildiğinde modül `programs.fish.interactiveShellInit`, `programs.zsh.initExtra` ve `programs.bash.initExtra` alanlarına `thornhill.sh` ve `thornhill.fish` köprülerini source eden satırları ekler. Kancalar yalnızca çıkış kodu sıfırdan farklı olan komutları raporlar; daemon çalışmıyorken hiçbir işlem yapmaz.

## 3. NixOS Sistem Modülü ile Entegrasyon

Alternatif olarak sistem düzeyinde kullanıcı servisi tanımlanabilir:

```nix
{ inputs, ... }:

{
  imports = [ inputs.thornhill.nixosModules.default ];

  services.thornhill.enable = true;
}
```

Bu modül, oturum açan her kullanıcı için `systemd.user.services.thornhilld` birimini üretir. Birim `graphical-session.target` ile bağlantılıdır ve şu sertleştirmeleri taşır:

| Direktif | Değer |
|---|---|
| `NoNewPrivileges` | `true` |
| `PrivateTmp` | `true` |
| `ProtectSystem` | `strict` |
| `ProtectHome` | `read-only` |
| `ReadWritePaths` | `%t/thornhill`, `%h/.local/state/thornhill` |

Servis yazma iznini yalnızca soket dizinine ve durum dizinine alır.

## 4. Yapılandırmanın Uygulanması

```bash
# Home Manager ile
home-manager switch

# NixOS sistem modülü ile
sudo nixos-rebuild switch
```

## 5. Servis Denetimi

```bash
systemctl --user status thornhilld
systemctl --user enable --now thornhilld
journalctl --user -u thornhilld -f
```

Servis aktifken soketin varlığı ve izinleri kontrol edilir:

```bash
ls -l "$XDG_RUNTIME_DIR/thornhill/thornhill.sock"
```

Beklenen izin: `srwx------`.

## 6. Ortam Değişkenleri

| Değişken | Varsayılan | Yer | İşlev |
|---|---|---|---|
| `THORNHILL_SOCK` | `$XDG_RUNTIME_DIR/thornhill/thornhill.sock` | kabuk kancaları | soket yolu override |
| `THORNHILL_S2_ENDPOINT` | `http://127.0.0.1:11434/api/generate` | `src/brain/mod.rs` | S2 sorgu adresi |
| `THORNHILL_S2_MODEL` | `llama3.2` | `src/brain/mod.rs` | kullanılacak model |
| `THORNHILL_S2_ON_BATTERY` | kapalı | `src/brain/mod.rs` | pil modunda S2'ye izin |
| `THORNHILL_S2_AUTO` | kapalı | `src/bus/server.rs` | S1'in kaçırdığı hataların otomatik S2'ye yükseltilmesi |
| `THORNHILL_HTTP_ADDR` | `127.0.0.1:9099` | `src/http/mod.rs` | web konsolu bağlama adresi |
| `THORNHILL_HTTP_ALLOW_REMOTE` | kapalı | `src/http/mod.rs` | loopback dışı bağlamaya izin |
| `XDG_STATE_HOME` | `~/.local/state` | `src/graph/db.rs` | graf veritabanı konumu |
| `XDG_RUNTIME_DIR` | `/run/user/$UID` | `src/bus/server.rs` | soket ve token konumu |
| `XDG_DATA_HOME` | `~/.local/share` | `src/bin/thornhilld.rs` | dashboard dosyalarının override dizini |
| `HYPRLAND_INSTANCE_SIGNATURE` | oturumdan | `src/sensory/hyprland.rs` | Hyprland oturum keşfi |

## 7. Quickshell Bağlama Örneği

`shell/ThornhillBridge.qml` bileşeni UDS üzerinden NDJSON konuşur ve çip listesini model olarak sunar:

```qml
// ThornhillBridge.qml ve ActionChipCard.qml ile aynı dizinde bulunur;
// QML, aynı dizindeki tipleri import olmadan örnekler.
import QtQuick

Item {
    ThornhillBridge {
        id: thornhillBridge
    }

    // Çip kartı listesi
    Column {
        Repeater {
            model: Object.keys(thornhillBridge.chips)
            delegate: ActionChipCard {
                required property string modelData
                chip: thornhillBridge.chips[modelData]
                bridge: thornhillBridge
            }
        }
    }

    Connections {
        target: thornhillBridge
        function onChipArrived(chipId) {
            console.log("yeni çip:", chipId)
        }
        function onChipExpired(chipId) {
            console.log("süresi doldu:", chipId)
        }
    }
}
```

`ThornhillBridge` bileşeninin arayüzü:

| Öğe | Tür | İşlev |
|---|---|---|
| `chips` | property, nesne | canlı çip listesi |
| `connected` | property, boole | UDS bağlantı durumu |
| `chipArrived` | sinyal | yeni çip bildirimi |
| `chipExpired` | sinyal | TTL süresi dolan çip |
| `actResult` | sinyal | `chip-act` yanıtı |
| `act(chipId, actionLabel, confirm)` | fonksiyon | çip eylemi gönderir |
| `ask(question)` | fonksiyon | S2 sorgusu gönderir |

## 8. Kurulum Sonrası Doğrulama

1. Servis durumu:

```bash
systemctl --user status thornhilld
```

2. Soket erişimi:

```bash
thornhill-ctl status
```

3. Graf ve web konsolu telemetrisi:

```bash
thornhill-ctl stats
curl -s http://127.0.0.1:9099/api/stats
```

4. Uçtan uca akış: bir terminalde dinleyici başlatılır, diğer terminalde hatalı bir komut çalıştırılır:

```bash
thornhill-ctl listen
```

```bash
false
```

Beklenen çıktı: `cmd.done` olayı ve ardından `chip.push` bildirimi. Web konsolunda `http://127.0.0.1:9099` adresinde aynı olayın görünmesi ve çip kartının listelenmesi akışın tamamlandığını gösterir.

5. S2 doğrulaması: yerel Ollama çalışırken:

```bash
thornhill-ctl ask "test"
```

6. Veri silme doğrulaması:

```bash
thornhill-ctl forget --before 1d
```
