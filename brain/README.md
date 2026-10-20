# brain/ — Bilişsel Katman (Sistem 2)

`memory/` — lessons/habits bakımı, retention politikası, unutma hakkı (`forget --before <ts>`).
`reasoning/` — S2 orkestratör: grafi ilgili alt zincirini **bağlam paketi** olarak hazırlar,
LLM'e (yerel/uzak, yapılandırılabilir) tek turlu, şemaya zorlanmış istek gönderir:
`{explanation, chips[], confidence}`.

## Kural

- Embedding araması yalnızca S2'de ve yapılandırılmışsa.
- S2 çağrıları rate-limited (1/10 sn/tetik sınıfı); pil modunda tamamen kapatılabilir.
- S1→S2 yükseltme kriterleri RFC-001 §9.3'te sabittir: yeni desen, conf<0.6+risk≥medium,
  kullanıcı isteği, ≥3 adımlı zincir, kural çelişkisi.
