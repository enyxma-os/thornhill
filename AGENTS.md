# AGENTS.md — Enyxma Thornhill Bilişsel Çekirdek Protokolü

Bu belge, `enyxma/thornhill` deposunda çalışacak yapay zeka ajanları ve geliştiriciler için değişmez mühendislik kurallarını belirler.

---

## 1. Temel Felsefe (The Core Axioms)

1. **Uzuv Değil, Toplam (Not a Limb, The Whole):**
   Thornhill reaktif bir sohbet botu veya dış eklenti değildir; Linux ortamının nabzını tutan, nedensellik kuran ve öngörü üreten yaşayan sistemik omurgadır.
2. **Sıfır Yükle Dinleme (Zero-Polling):**
   Asla periyodik `while true; sleep` döngüleri çalıştırılamaz. Tüm duyu olayları Hyprland `socket2`, Linux netlink proc connector veya shell hook mekanizmalarıyla olay güdümlü (event-driven) olmalıdır.
3. **Çift Zihin Disiplini (Dual-Mind):**
   - **Sistem 1 (Refleks):** 0 MB VRAM, <1 ms yanıt süresi. Deterministik kurallar ve kural tabloları ile çözülür; LLM çağrılmaz.
   - **Sistem 2 (Biliş):** Yalnızca RFC-001 §9.3'teki 6 katı yükseltme kriterinde (açıklanamayan yeni hata, <0.6 güven, doğrudan kullanıcı sorgusu) devreye girer.
4. **Action Chips (Eylem Çipleri):**
   Thornhill kullanıcıya uzun paragraflar dökmek yerine, doğrudan tıklanabilir veya onaylanabilir `Action Chip` çıktıları üretir.
5. **Evrensel Taşınabilirlik (ADR-062):**
   Kod içinde asla sabit kullanıcı adı (`/home/<user>/`) veya hardcoded yollar bulunamaz. Tüm yollar `$XDG_RUNTIME_DIR` ve `$XDG_STATE_HOME` üzerinden çözümlenir.

---

## 2. Dizin Mimarisi

- `daemon/`: Arka plan servisi (Sensory Tap, SQLite Causal Graph, UDS NDJSON bus).
- `brain/`: Bilişsel katman (Localmind Zihin Sarayı entegrasyonu, S2 orkestrasyonu).
- `shell-hook/`: Bash ve Zsh için hafif, sıfır-maliyetli hata kancaları.
- `docs/`: RFC mimari kararları (RFC-001 Anayasası).

---

## 3. Doğrulama ve Kalite Standartları

Her commit öncesi:
- `just verify` hatasız tamamlanmalıdır.
- Sabit yol veya sızıntı içermemelidir.
- SQLite WAL moduna ve veri tutma (retention) kurallarına sadık kalınmalıdır.
