// ThornhillBridge.qml — Faz 4: Quickshell ↔ Thornhill UDS köprüsü
//
// RFC-001 §7 tel protokolü (NDJSON, satır sınırlı) konuşur:
//   - Bağlanınca `hello` + `sub{filter:{chips:true}}` gönderir
//   - `chip.push` bildirimlerini ActionChip modeline çevirir
//   - `chip.act{chip_id, action_label, confirm}` yazar (§7.2, §8.5)
//
// Dayanıklılık (denetim kararı #4): daemon yeniden başlarsa üstel geri
// çekilmeli yeniden bağlanma (1 sn → 30 sn üst sınır).
//
// Doğrulama notu: Quickshell.Io.Socket / SplitParser API'si
// https://quickshell.org/docs/v0.3.1/types/Quickshell.Io/Socket/ temel alınmıştır;
// Quickshell sürümünüzde özellik adları farklıysa (path/target) aşağıda işaretlidir.

import QtQuick
import Quickshell
import Quickshell.Io

QtObject {
    id: root

    // ---- Genel arayüz (panel/widget bunları tüketir) ----
    // Canlı çip listesi: [{id, title, why, trigger, labels[], actions[], ttlMs, deadline, conf}]
    property var chips: ({})
    property bool connected: false

    signal chipArrived(string chipId)
    signal chipExpired(string chipId)
    signal actResult(string chipId, var response)

    // ---- Yapılandırma (ADR-062: sabit yol yok) ----
    readonly property string runtimeDir: Quickshell.env("XDG_RUNTIME_DIR") || "/run/user/1000"
    readonly property string socketPath: (Quickshell.env("THORNHILL_SOCK") || (runtimeDir + "/thornhill/thornhill.sock"))

    // ---- UDS soketi ----
    property bool shouldConnect: true
    property int backoffMs: 1000

    property Socket _socket: Socket {
        // Not: Quickshell sürümünüze göre özellik adı `path` veya `target` olabilir
        path: root.socketPath
        connected: root.shouldConnect

        parser: SplitParser {
            // NDJSON: satır sınırlı tel protokolü (RFC-001 §7)
            onSplitData: data => root._onLine(data)
        }

        onConnectedChanged: {
            root.connected = connected
            if (connected) {
                root.backoffMs = 1000
                root._handshake()
            } else {
                root._scheduleReconnect()
            }
        }
    }

    // ---- TTL süpürme (§7.2: süresi dolan çip düşer) ----
    property Timer _pruneTimer: Timer {
        interval: 1000
        running: true
        repeat: true
        onTriggered: root._pruneExpired()
    }

    // ---- Genel fonksiyonlar ----

    // risk=none: tek tık (confirm=true). risk=low/medium: önce önizleme, sonra hold.
    function act(chipId, actionLabel, confirm) {
        const msg = {
            op: "chip.act",
            req: "ui-" + Date.now(),
            chip_id: chipId,
            action_label: actionLabel,
            confirm: confirm
        }
        root._send(JSON.stringify(msg))
    }

    function ask(question) {
        root._send(JSON.stringify({ op: "ask", req: "ui-" + Date.now(), question: question }))
    }

    // ---- İç implementasyon ----

    function _send(line) {
        if (!root.connected) {
            console.warn("[thornhill] soket bağlı değil; mesaj gönderilemedi")
            return
        }
        // Not: sürüme göre socket.writer.write() / socket.write()
        root._socket.writer.write(line + "\n")
    }

    function _handshake() {
        root._send(JSON.stringify({
            op: "hello", req: "ui-hello", client: "enyxma-shell", v: 1
        }))
        root._send(JSON.stringify({
            op: "sub", req: "ui-sub", filter: { kinds: [], srcs: [], chips: true }
        }))
    }

    function _onLine(data) {
        let msg
        try {
            msg = JSON.parse(data)
        } catch (e) {
            console.warn("[thornhill] bozuk NDJSON atlandı:", e)
            return
        }

        if (msg.op === "chip.push" && msg.chip) {
            const c = msg.chip
            const entry = {
                id: c.id,
                title: c.title || "",
                why: c.why || "",
                trigger: c.trigger || "",
                labels: c.labels || [],
                actions: c.actions || [],
                ttlMs: c.ttl_ms || 60000,
                conf: c.conf !== undefined ? c.conf : 1.0,
                // Widget tarafı son kullanma anı (daemon ile senkron, §7.2)
                deadline: Date.now() + (c.ttl_ms || 60000),
                // §2 denetim kararı: medium riskli eylemde komut tam gösterilir
                execCmd: (c.actions || []).filter(a => a.action_type === "exec")
                            .map(a => a.payload && a.payload.cmd ? a.payload.cmd : "").join("")
            }
            const next = Object.assign({}, root.chips)
            next[c.id] = entry
            root.chips = next
            root.chipArrived(c.id)
        } else if (msg.op === "event.push") {
            // v1 widget olay akışını göstermez; yalnız çipler (NG1: Thornhill konuşmaz)
        } else if (msg.ok === false && msg.error) {
            console.warn("[thornhill] daemon hatası:", msg.error)
        } else {
            // chip.act yanıtları vs. — ilgilenen kartlara duyuralım
            root.actResult(msg.chip_id || "", msg)
        }
    }

    function _pruneExpired() {
        const now = Date.now()
        const next = {}
        let changed = false
        for (const id in root.chips) {
            if (root.chips[id].deadline > now) {
                next[id] = root.chips[id]
            } else {
                changed = true
                root.chipExpired(id)
            }
        }
        if (changed)
            root.chips = next
    }

    function _scheduleReconnect() {
        // Üstel geri çekilme (bağlantı yönetimi; polling değil — Aksiyom 2)
        const wait = root.backoffMs
        root.backoffMs = Math.min(root.backoffMs * 2, 30000)
        Qt.callLater(function () {
            Timer {
                interval: wait
                running: true
                onTriggered: {
                    if (!root.connected && root.shouldConnect)
                        root._socket.connected = true
                }
            }
        })
    }
}
