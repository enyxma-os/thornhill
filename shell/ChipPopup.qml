// ChipPopup.qml — Enyxma Shell'e en az dokunuşla bağlanan örnek katman
//
// Kullanım: mevcut shell.qml'nize şu üç parça eklenir:
//   1. Bridge örneği (tekil, uygulama ömrü boyunca)
//   2. Kart yığını (sağ alt köşe; yeni çip gelince belirir, TTL'de düşer)
//   3. İsteğe bağlı: ask() ile Super+S sorgu kutusu (§9.3.3)
//
// Bu dosya bağımsız bir pencere olarak da çalışır (test modu):
//   quickshell -p shell/ChipPopup.qml

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland

ShellRoot {
    id: shellRoot

    ThornhillBridge {
        id: thornhillBridge
    }

    // ---- Çip yığını (ekranın sağ altı) ----
    PanelWindow {
        anchors.bottom: true
        anchors.right: true
        margins.bottom: 48
        margins.right: 16

        implicitWidth: 420
        // Yükseklik içerikle büyüsün: görünür kart sayısına göre
        implicitHeight: chipColumn.implicitHeight + (chipColumn.children.length > 0 ? 24 : 0)
        color: "transparent"

        ColumnLayout {
            id: chipColumn
            anchors.fill: parent
            spacing: 8

            Repeater {
                // Yeni çipler üste: Object.keys ters sırayla
                model: Object.keys(thornhillBridge.chips).reverse()

                ActionChipCard {
                    required property string modelData
                    chip: thornhillBridge.chips ? (thornhillBridge.chips[modelData] || null) : null
                    bridge: thornhillBridge
                    Layout.fillWidth: true
                }
            }

            // Boşta iken pencere hiç yer kaplamasın
            Item { visible: chipColumn.children.length === 0; implicitHeight: 0 }
        }
    }

    // ---- Bağlantı durumu göstergesi (hata ayıklama; istenirse kaldırın) ----
    PanelWindow {
        anchors.top: true
        anchors.right: true
        margins.top: 8
        margins.right: 16
        implicitWidth: eyeLabel.implicitWidth + 16
        implicitHeight: eyeLabel.implicitHeight + 8
        color: "transparent"

        Label {
            id: eyeLabel
            text: thornhillBridge.connected ? "👁️ Thornhill" : "💤 Thornhill"
            color: thornhillBridge.connected ? "#98c379" : "#5c6370"
            font.pixelSize: 13
        }
    }
}
