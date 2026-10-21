// ActionChipCard.qml — tekil Action Chip kartı (RFC-001 §7.2 görsel karşılığı)
//
// §8.5 onay merdiveninin UI karşılığı:
//   - risk "none":  tek tık → confirm:true
//   - risk "low"/"medium": ilk tık önizleme (confirm:false, daemon hiçbir şey
//     çalıştırmaz), ardından 1.5 sn BASILI TUTMA → confirm:true
//   - "high": hiçbir buton üretilmez (daemon zaten reddeder)
// Denetim kararı #3: exec eyleminde komut metni TAMAMI kartta görünür.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: card

    property var chip: null           // ThornhillBridge.chips[id] girdisi
    property var bridge: null         // ThornhillBridge örneği
    property bool previewing: false   // "hold-to-confirm" evresi

    visible: chip !== null && chip !== undefined
    radius: 10
    color: "#1e2229"
    border.color: previewing ? "#e0af68" : "#3b4261"
    border.width: previewing ? 2 : 1

    implicitHeight: (!card.chip) ? 0 : (layout.implicitHeight + 24)

    ColumnLayout {
        id: layout
        anchors.fill: parent
        anchors.margins: 12
        spacing: 6
        visible: card.chip !== null

        RowLayout {
            Layout.fillWidth: true
            Label {
                Layout.fillWidth: true
                text: card.chip ? (card.chip.title || "") : ""
                color: "#e06c75"
                font.bold: true
                elide: Text.ElideRight
                maximumLineCount: 2
                wrapMode: Text.Wrap
            }
            Label {
                text: "güven " + Math.round((card.chip ? (card.chip.conf || 1.0) : 1.0) * 100) + "%"
                color: "#5c6370"
                font.pixelSize: 10
            }
        }

        Label {
            Layout.fillWidth: true
            text: card.chip ? (card.chip.why || "") : ""
            color: "#abb2bf"
            font.pixelSize: 11
            wrapMode: Text.Wrap
        }

        // Denetim kararı: exec komutu tam ve mono gösterilir (gizli enjeksiyon yok)
        Label {
            visible: card.chip ? ((card.chip.execCmd || "") !== "") : false
            Layout.fillWidth: true
            text: "⚑ çalıştırılacak: " + (card.chip ? card.chip.execCmd : "")
            color: "#e0af68"
            font.family: "monospace"
            font.pixelSize: 10
            wrapMode: Text.WrapAnywhere
        }

        // TTL ilerleme çubuğu (§7.2: ttl dolunca kart kendiliğinden düşer)
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 3
            color: "#3b4261"
            Rectangle {
                id: ttlBar
                property real now: Date.now()
                property real remain: (card.chip && card.chip.ttlMs) 
                    ? Math.max(0, Math.min(1, (card.chip.deadline - now) / card.chip.ttlMs))
                    : 0
                anchors.left: parent.left
                height: parent.height
                width: parent.width * remain
                color: "#61afef"
                // 1 sn'de bir yeniden hesap (görsel geri sayım)
                Timer {
                    interval: 1000
                    running: true
                    repeat: true
                    onTriggered: ttlBar.now = Date.now()
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            Repeater {
                model: (card.chip && card.chip.labels) ? card.chip.labels : []

                Button {
                    required property string modelData
                    required property int index
                    text: {
                        if (!card.previewing)
                            return modelData
                        const a = card.chip ? card.chip.actions[index] : null
                        return (a && (a.risk === "low" || a.risk === "medium"))
                            ? modelData + " (basılı tut)" : modelData
                    }
                    Layout.fillWidth: true

                    // risk=medium için basılı tutma jesti (§8.5 hold-to-confirm)
                    MouseArea {
                        id: hold
                        anchors.fill: parent
                        property bool holding: false

                        onPressed: {
                            const a = card.chip ? card.chip.actions[index] : null
                            if (a && (a.risk === "low" || a.risk === "medium")) {
                                if (!card.previewing) {
                                    card.previewing = true   // 1. tık: yalnız önizleme
                                } else {
                                    holding = true           // 2. evre: basılı tutuluyor
                                    holdTimer.start()
                                }
                            }
                        }
                        onReleased: holding = false
                        onClicked: {
                            const a = card.chip ? card.chip.actions[index] : null
                            if (a && a.risk === "none") {
                                if (card.bridge)
                                    card.bridge.act(card.chip.id, modelData, true) // risk none: anında
                            }
                        }

                        Timer {
                            id: holdTimer
                            interval: 1500
                            onTriggered: {
                                if (hold.holding) {
                                    card.previewing = false
                                    if (card.bridge)
                                        card.bridge.act(card.chip.id, modelData, true)
                                }
                            }
                        }
                    }
                }
            }
        }

        Label {
            visible: card.previewing
            text: "İkinci onay bekleniyor — onaylamak için düğmeyi 1.5 sn basılı tutun"
            color: "#e0af68"
            font.pixelSize: 10
        }
    }
}
