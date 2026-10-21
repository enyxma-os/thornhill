# shell-hook/ — Kabuk Köprüleri (RFC-001 §4.1, v2)

Terminal çıkış kodlarını duyu hattına bağlayan sıfır maliyetli kancalar.

## v2 — ENXIO düzeltmesi

Önceki sürüm `exec 9>>"$sock"` ile sokete doğrudan fd yönlendirmesi yapıyordu.
Bu **yalnız zsh'ta** çalışır; bash ve fish'te Linux VFS, `SOCK_STREAM` soketine
`open()` izin vermez (`ENXIO`). v2'de her üç kabuk aynı deseni kullanır:

> Hata anında (`exit_code != 0`) `thornhill-ctl inject` **arka planda** çalışır.
> Soket/araç yoksa 0 ms'de sessizce pas geçer; kabuk asla bloklanmaz.

## Dosyalar

| Dosya | Kabuk | Mekanizma |
|---|---|---|
| `thornhill.sh` | zsh + bash | zsh: `preexec`+`precmd` hook · bash: `DEBUG` trap + `PROMPT_COMMAND` |
| `thornhill.fish` | fish | `fish_preexec` / `fish_postexec` olayları |

## Kurulum

Home Manager modülü ile otomatik (`services.thornhill.enableShellHook = true`)
veya elle:

```fish
# config.fish:
source <paket>/share/thornhill/shell-hook/thornhill.fish
```

```zsh
# .zshrc:
source <paket>/share/thornhill/shell-hook/thornhill.sh
```

## Notlar

- `cmd` alanı 200 baytla kesilir ve JSON'a güvenli gömülür (ters bölü/tırnak/satır sonları temizlenir).
- Sadece hatalı komutlar raporlanır (gürültü kontrolü, RFC-001 §4.1).
- `THORNHILL_SOCK` ortam değişkeni ile soket yolu override edilebilir (varsayılan: `$XDG_RUNTIME_DIR/thornhill/thornhill.sock`, ADR-062).
