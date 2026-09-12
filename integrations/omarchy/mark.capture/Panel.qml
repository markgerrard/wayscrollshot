import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
    id: root
    moduleName: "mark.capture"
    ipcTarget: "mark.capture"
    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight
    property int selected: 0
    property bool cloudView: false
    property var cloudItems: []
    property string cloudMessage: ""
    property var pendingArgs: []
    readonly property var rows: [
        {label: "All-in-One", hint: "Super Shift Print", icon: 0, args: ["--capture-bar"]},
        {label: "Capture Area", hint: "", icon: 0, args: ["--screenshot"]},
        {label: "Capture Fullscreen", hint: "", icon: 5, args: ["--screenshot", "--select-mode", "screen"]},
        {label: "Capture Window", hint: "", icon: 6, args: ["--screenshot", "--select-mode", "window"]},
        {label: "Scrolling Capture", hint: "Super Alt R", icon: 7, args: []},
        {label: "View Cloud", hint: "", icon: 5, cloud: true}
    ]

    function activate(index) {
        const row = rows[index]
        if (row.cloud) {
            cloudView = true
            selected = 0
            refreshCloud()
            return
        }
        pendingArgs = [Quickshell.env("HOME") + "/.local/bin/wayscrollshot", "--toggle"].concat(row.args)
        close()
        launchDelay.restart()
    }
    function refreshCloud() {
        if (cloudPoll.running) return
        cloudMessage = "Loading captures…"
        cloudPoll.running = true
    }
    function restoreCapture(index) {
        if (index < 0 || index >= cloudItems.length) return
        const capture = cloudItems[index]
        pendingArgs = [Quickshell.env("HOME") + "/.local/bin/wayscrollshot", "--cloud-gallery", "--cloud-id", String(capture.id)]
        close()
        launchDelay.restart()
    }
    function moveCloud(direction) {
        if (cloudItems.length === 0) return
        selected = Math.max(0, Math.min(cloudItems.length - 1, selected + direction))
        cloudStrip.positionViewAtIndex(selected, ListView.Contain)
    }
    function captureDate(seconds) {
        return Qt.formatDateTime(new Date(seconds * 1000), "dd MMM · HH:mm")
    }

    onOpenedChanged: if (!opened) { cloudView = false; selected = 0 }

    Timer { id: launchDelay; interval: 180; onTriggered: Quickshell.execDetached(root.pendingArgs) }
    Process {
        id: cloudPoll
        command: [Quickshell.env("HOME") + "/.local/bin/wayscrollshot", "--cloud-list-json"]
        stdout: StdioCollector {
            waitForEnd: true
            onStreamFinished: {
                try {
                    root.cloudItems = JSON.parse(String(text || "[]"))
                    root.cloudMessage = root.cloudItems.length === 0 ? "No cloud captures yet" : ""
                    root.selected = 0
                } catch (error) {
                    root.cloudItems = []
                    root.cloudMessage = "Could not read cloud captures"
                }
            }
        }
        onExited: function(code) { if (code !== 0) root.cloudMessage = "Could not read cloud captures" }
    }

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
        contentWidth: popup.fittedContentWidth(Style.space(root.cloudView ? 760 : 330))
        contentHeight: popup.fittedContentHeight(root.cloudView ? Style.space(300) : menuColumn.implicitHeight, Style.space(420))

        PanelKeyCatcher {
            id: keys
            anchors.fill: parent
            onMoveRequested: function(dx, dy) {
                if (root.cloudView) root.moveCloud(dx !== 0 ? dx : dy)
                else root.selected = Math.max(0, Math.min(root.rows.length - 1, root.selected + dy))
            }
            onActivateRequested: root.cloudView ? root.restoreCapture(root.selected) : root.activate(root.selected)
            onCloseRequested: {
                if (root.cloudView) { root.cloudView = false; root.selected = 0 }
                else root.close()
            }
            onTabRequested: function(direction) { root.switchPanel(direction) }

            Column {
                id: menuColumn
                visible: !root.cloudView
                width: parent.width
                spacing: Style.space(3)
                Repeater {
                    model: root.rows
                    delegate: Rectangle {
                        required property var modelData
                        required property int index
                        width: menuColumn.width
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

            Item {
                visible: root.cloudView
                anchors.fill: parent

                Row {
                    id: cloudHeader
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    height: Style.space(40)
                    spacing: Style.space(10)
                    Rectangle {
                        width: Style.space(70); height: Style.space(32); radius: Style.cornerRadius
                        color: backMouse.containsMouse ? Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.12) : "transparent"
                        Text { anchors.centerIn: parent; text: "‹  Back"; color: Color.foreground; font.family: Style.font.family; font.pixelSize: Style.font.caption }
                        MouseArea { id: backMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { root.cloudView = false; root.selected = 0 } }
                    }
                    Text {
                        width: parent.width - Style.space(160); anchors.verticalCenter: parent.verticalCenter
                        text: "Cloud captures" + (root.cloudItems.length ? "  ·  " + root.cloudItems.length : "")
                        color: Color.foreground; font.family: Style.font.family; font.pixelSize: Style.font.heading; font.bold: true
                    }
                    Rectangle {
                        width: Style.space(70); height: Style.space(32); radius: Style.cornerRadius
                        color: refreshMouse.containsMouse ? Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.12) : "transparent"
                        Text { anchors.centerIn: parent; text: "Refresh"; color: Color.foreground; font.family: Style.font.family; font.pixelSize: Style.font.caption }
                        MouseArea { id: refreshMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.refreshCloud() }
                    }
                }

                ListView {
                    id: cloudStrip
                    anchors.left: parent.left; anchors.right: parent.right; anchors.top: cloudHeader.bottom; anchors.bottom: parent.bottom
                    orientation: ListView.Horizontal
                    model: root.cloudItems
                    spacing: Style.space(10)
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    visible: root.cloudItems.length > 0
                    delegate: Rectangle {
                        required property var modelData
                        required property int index
                        width: Style.space(184)
                        height: cloudStrip.height - Style.space(8)
                        radius: Style.cornerRadius
                        color: root.selected === index ? Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.14) : Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.055)
                        border.width: root.selected === index ? 1 : 0
                        border.color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.28)
                        clip: true
                        Image {
                            anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top; anchors.bottom: cardCaption.top
                            anchors.margins: Style.space(7)
                            source: modelData.thumbnail
                            fillMode: Image.PreserveAspectFit
                            asynchronous: true
                            smooth: true
                            cache: false
                        }
                        Column {
                            id: cardCaption
                            anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.margins: Style.space(9)
                            height: Style.space(42); spacing: Style.space(2)
                            Text { text: root.captureDate(modelData.created_at); color: Color.foreground; font.family: Style.font.family; font.pixelSize: Style.font.caption; elide: Text.ElideRight; width: parent.width }
                            Text { text: modelData.width + " × " + modelData.height; color: Qt.darker(Color.foreground, 1.5); font.family: Style.font.family; font.pixelSize: Style.font.caption * 0.9 }
                        }
                        MouseArea {
                            anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                            onEntered: root.selected = index
                            onClicked: root.restoreCapture(index)
                        }
                    }
                }
                Text {
                    visible: root.cloudItems.length === 0
                    anchors.centerIn: parent
                    text: root.cloudMessage
                    color: Qt.darker(Color.foreground, 1.35)
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                }
            }
        }
    }
}
