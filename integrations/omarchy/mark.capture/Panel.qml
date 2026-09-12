import QtQuick
import Quickshell
import qs.Commons
import qs.Ui

Panel {
    id: root
    moduleName: "mark.capture"
    ipcTarget: "mark.capture"
    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight
    property int selected: 0
    property var pendingArgs: []
    readonly property var rows: [
        {label: "All-in-One", hint: "Super Shift Print", icon: 0, args: ["--capture-bar"]},
        {label: "Capture Area", hint: "", icon: 0, args: ["--screenshot"]},
        {label: "Capture Fullscreen", hint: "", icon: 5, args: ["--screenshot", "--select-mode", "screen"]},
        {label: "Capture Window", hint: "", icon: 6, args: ["--screenshot", "--select-mode", "window"]},
        {label: "Scrolling Capture", hint: "Super Alt R", icon: 7, args: ["--auto-scroll"]}
    ]
    function activate(index) {
        pendingArgs = [Quickshell.env("HOME") + "/.local/bin/wayscrollshot", "--toggle"].concat(rows[index].args)
        close()
        launchDelay.restart()
    }
    Timer { id: launchDelay; interval: 180; onTriggered: Quickshell.execDetached(root.pendingArgs) }
    component CaptureIcon: Image {
        property int glyph: 0
        source: glyph < 6 ? "capture-icons.png" : "capture-modes.png"
        sourceClipRect: {
            const boxes = [[120,260,240,260],[515,260,240,260],[925,270,215,245],[125,760,215,225],[525,760,215,225],[905,750,250,250],[170,160,590,590],[1080,180,530,530]]
            const b = boxes[glyph]
            return Qt.rect(b[0], b[1], b[2], b[3])
        }
        fillMode: Image.PreserveAspectFit
        smooth: true
        width: Style.space(20)
        height: Style.space(20)
    }
    BarIconButton {
        id: button
        anchors.fill: parent
        bar: root.bar
        text: " "
        CaptureIcon { anchors.centerIn: parent; width: Style.space(16); height: Style.space(16) }
        onPressed: root.toggle()
    }
    KeyboardPanel {
        id: popup
        anchorItem: button
        owner: root
        bar: root.bar
        open: root.opened
        focusTarget: keys
        contentWidth: popup.fittedContentWidth(Style.space(330))
        contentHeight: popup.fittedContentHeight(column.implicitHeight, Style.space(390))
        PanelKeyCatcher {
            id: keys
            anchors.fill: parent
            onMoveRequested: function(dx, dy) { root.selected = Math.max(0, Math.min(root.rows.length - 1, root.selected + dy)) }
            onActivateRequested: root.activate(root.selected)
            onCloseRequested: root.close()
            onTabRequested: function(direction) { root.switchPanel(direction) }
            Column {
                id: column
                width: parent.width
                spacing: Style.space(3)
                Repeater {
                    model: root.rows
                    delegate: Rectangle {
                        required property var modelData
                        required property int index
                        width: column.width
                        height: Style.space(44)
                        radius: Style.cornerRadius
                        color: root.selected === index ? Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.10) : "transparent"
                        CaptureIcon { glyph: modelData.icon; anchors.left: parent.left; anchors.leftMargin: Style.space(10); anchors.verticalCenter: parent.verticalCenter }
                        Text { text: modelData.label; anchors.left: parent.left; anchors.leftMargin: Style.space(42); anchors.verticalCenter: parent.verticalCenter; color: Color.foreground; font.family: Style.font.family; font.pixelSize: Style.font.body }
                        Text { text: modelData.hint; anchors.right: parent.right; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter; color: Qt.darker(Color.foreground, 1.6); font.family: Style.font.family; font.pixelSize: Style.font.caption * 0.85 }
                        MouseArea { anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onEntered: root.selected = index; onClicked: root.activate(index) }
                    }
                }
            }
        }
    }
}
