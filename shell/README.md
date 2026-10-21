# shell/ — Faz 4: Enyxma Shell Action Chip katmanı (Quickshell/QML)

Thornhill daemon'unun ürettiği Action Chip'leri masaüstünde gösteren, tıklamayla
`chip.act` gönderen görsel katman.

## Dosyalar

| Dosya | Görev |
|---|---|
| `ThornhillBridge.qml` | UDS + NDJSON köprüsü: hello/sub handshake, chip.push → model, act()/ask(), üstel geri çekilmeli yeniden bağlanma, TTL süpürme |
| `ActionChipCard.qml` | Tekil çip kartı: başlık, gerekçe (why), güven, TTL geri sayımı, §8.5 onay merdiveni jestleri |
| `ChipPopup.qml` | Bağışıklı entegrasyon örneği: sağ alt köşede çip yığını + bağlantı durumu gözü |

## Hızlı test

```bash
thornhilld &                      # daemon
quickshell -p shell/ChipPopup.qml # widget (ayrı süreç olarak da çalışır)
thornhill-ctl inject --src shell --kind cmd.done --subject test \
  --ctx '{"exit_code": 101, "cmd": "cargo build", "error": "E0432"}'
# Beklenen: sağ altta "Komut hatası (exit 101): cargo build" kartı,
# TTL ilerledikçe incelen mavi çubuk, 60 sn sonunda kendiliğinden kaybolur.
```

## §8.5 onay merdiveninin UI karşılığı

| risk | jest |
|---|---|
| `none` | tek tık (anında) |
| `low` / `medium` | 1. tık: önizleme (daemon hiçbir şey çalıştırmaz) → 2. evre: düğmeyi **1.5 sn basılı tut** |
| `high` | buton üretilmez (daemon zaten reddeder) |

Exec eylemlerinde çalıştırılacak komut **tamamıyla** kartta görünür (denetim
kararı: gizli enjeksiyon yok).

## Kurulum (enyxma/shell'e gömme)

1. Bu üç dosyayı `enyxma/shell` deposuna kopyalayın.
2. Ana `shell.qml`'nize `ThornhillBridge { id: bridge }` ekleyip çip yığınını
   `ChipPopup.qml`'deki gibi Repeater ile bağlayın (veya ChipPopup'ı ShellRoot
   olarak ayrı quickshell örneği çalıştırın).
3. `thornhill-ctl`'in PATH'te olması şart değil — köprü doğrudan UDS konuşur.

## Bilinen sürüm uyarıları

- `Socket.path` / `Socket.connected` özellik adları Quickshell 0.3.x'e göredir;
  sürümünüzde `target` / `connected` farklıysa ThornhillBridge.qml'deki
  işaretli satırları uyarlayın (bkz. https://quickshell.org/docs/v0.3.1/types/Quickshell.Io/Socket/).
- `SplitParser` satır sınırlı veri verir; NDJSON protokolü bunu varsayar.
