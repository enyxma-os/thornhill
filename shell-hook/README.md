# shell-hook/ — Kabuk Köprüsü (RFC-001 §4.1)

Terminal çıkış kodlarını duyu hattına bağlayan sıfır maliyetli kancalar.

- `thornhill.sh` — zsh `precmd` + bash `PROMPT_COMMAND` köprüsü
- Kural: yalnız `exit_code != 0` iken UDS'ye yazar (gürültü kontrolü)
- Soket yoksa / fd kapalıysa **sessizce pas geçer** — Thornhill olmadan kabuk asla bozulmaz
- ADR-062: soket yolu `$XDG_RUNTIME_DIR/thornhill/thornhill.sock`'ten çözülür

## Kurulum

```zsh
# .zshrc'ye:
source /nix/store/...-thornhill-0.1.0/share/thornhill/shell-hook/thornhill.sh
```

Bunun için `flake.nix`'in `installPhase`'ine `cp shell-hook/thornhill.sh $out/share/thornhill/shell-hook/` eklenmelidir (Faz 2 sonrası paketleme turunda yapılacak).
