# Thornhill

> **Bir uzuv sorulunca çalışır. Thornhill hiç sorulmadığı halde bakar.**

`enyxma` organizasyonu altında bağımsız, evrensel bir bilişsel omurga.
Anayasa: [`docs/rfc-001.md`](docs/rfc-001.md)

## Yapı (RFC-001 §11)

```
├── flake.nix              # Saf Nix paketleme (nixos-config'e dokunulmaz)
├── Cargo.toml             # Motor: Rust (S1 + duyu + bus) — RFC-002'de kilitlenir
├── daemon/
│   ├── sensory/           # Hyprland socket2, inotify, proc connector, udev
│   ├── graph/             # SQLite WAL, nedensellik grafiği DDL
│   └── bus/               # UDS sunucusu (NDJSON tel protokolü)
├── brain/
│   ├── memory/            # lessons/habits bakımı, retention
│   └── reasoning/         # S2 orkestratör, bağlam paketleyici
├── shell-hook/            # zsh precmd / bash PROMPT_COMMAND köprüsü
└── docs/
    └── rfc-001.md
```

## Dört Sütun

1. **Duyusal Algı (Sensory Tap)** — sıfır-yük, poll'suz olay dinleme
2. **Nedensellik Grafiği** — "X → Y → çözüm Z" zincirleri (SQLite)
3. **Sessiz Eylem Motoru** — Action Chip'ler, onay merdiveni
4. **İki Zihinli Beyin** — S1 refleks (<1 ms, 0 VRAM) / S2 biliş (LLM)

## Kurallar (özet — tam liste RFC-001'de)

- Kod yazılmadan önce RFC kararlanır.
- S1, LLM'e hiç sormaz; yükseltme kriterleri §9.3'te sabittir.
- `chip.act` onayı olmadan hiçbir yazma eylemi çalışmaz (§8).
- Varsayılan: veri makineden çıkmaz (§8).
