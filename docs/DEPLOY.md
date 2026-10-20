# Thornhill Dağıtım Kılavuzu (Seçenek 1)

Amaç: Thornhill'i makinenizde **kalıcı, ambiyanal bir servis** haline getirmek —
elle başlatmadan, her girişte otomatik.

## A) Home Manager ile (önerilen — her şey kullanıcı düzeyinde)

`home.nix` (veya yapılandırma dosyanız) içine:

```nix
{
  imports = [ inputs.thornhill.homeManagerModules.default ];

  services.thornhill = {
    enable = true;
    enableShellHook = true;   # zsh/bash'e hata kancasını source'lar
  };
}
```

`thornhill` flake girdisini eklemeyi unutmayın:

```nix
inputs.thornhill.url = "git+https://codeberg.org/enyxma/thornhill";
```

Uygulama:

```bash
home-manager switch
systemctl --user status thornhilld     # aktif olmalı
```

## B) NixOS Modülü ile (system düzeyi kullanıcı servisi)

`configuration.nix` içine:

```nix
{
  imports = [ inputs.thornhill.nixosModules.default ];

  services.thornhill.enable = true;
}
```

Uygulama:

```bash
sudo nixos-rebuild switch
systemctl --user status thornhilld
```

Not: `nixosModules` sürümü `graphical-session.target`'e bağlanır — Wayland
oturumunuz açıldığında daemon başlar, kapanınca durur.

## C) Doğrulama: ambiyanal hattın canlılığı

Servis aktifken (elle başlatmadan):

```bash
# 1) Dinleyiciyi bir terminalde açın
thornhill-ctl listen

# 2) Başka bir terminalde gerçek bir hata üretin
false

# 3) Dinleyicide beklenen çıktı:
#    [event.push] ... | src=shell kind=cmd.done subject=... conf=1.00
#    [chip.push] ... | Komut hatası (exit 1): false
#    çipler: Hatayı Açıkla | Komutu Yeniden Çalıştır
```

Bu üç adım çalışıyorsa sistem tam anlamıyla ambiyanaldır: hiçbir şeyi elle
çağırmadan hatayı gördü, grafe yazdı, refleks gösterdi.

## D) Servis sertleştirmesi (otomatik uygulanır)

NixOS modülü şu systemd kısıtlamalarıyla gelir (RFC-001 §8 uyumlu):

- `NoNewPrivileges`, `PrivateTmp`, `ProtectSystem=strict`, `ProtectHome=read-only`
- Yazma izni yalnız `%t/thornhill` (soket) ve `~/.local/state/thornhill` (graf)

## E) Yedekleme / Temizleme

- Graf yolu: `$XDG_STATE_HOME/thornhill/graph.db` (WAL)
- Unutma hakkı (RFC-001 §8.6): `sqlite3 $XDG_STATE_HOME/thornhill/graph.db "DELETE FROM events WHERE ts < epoch_ms"` (retention CLI Faz 3'te `thornhill-ctl forget` olarak eklenecek)
- Servis günlükleri: `journalctl --user -u thornhilld -f`
