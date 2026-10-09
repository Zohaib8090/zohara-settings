/*
    SPDX-License-Identifier: GPL-3.0-or-later
    Zohara OS: a video that plays on a loop as the desktop background.
*/

import QtQuick
import QtMultimedia
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root

    readonly property string path: root.configuration.VideoPath || ""
    readonly property int fill: root.configuration.FillMode || 0
    readonly property real speed: Math.max(0.25, Math.min(2, root.configuration.Speed || 1))

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    MediaPlayer {
        id: player
        source: root.path !== "" ? "file://" + root.path : ""
        loops: MediaPlayer.Infinite
        playbackRate: root.speed
        videoOutput: output
        audioOutput: AudioOutput {
            muted: root.configuration.Muted !== false
            volume: 0.6
        }
        onSourceChanged: if (source != "" && root.visible) play()
        onErrorOccurred: (error, message) => console.warn("Live wallpaper:", message)
    }

    VideoOutput {
        id: output
        anchors.fill: parent
        fillMode: root.fill === 1 ? VideoOutput.PreserveAspectFit : (root.fill === 2 ? VideoOutput.Stretch : VideoOutput.PreserveAspectCrop)
    }

    // Do not decode video nobody can see (the desktop is hidden, or the screen is off).
    onVisibleChanged: {
        if (visible && player.source != "") {
            player.play();
        } else {
            player.pause();
        }
    }

    Component.onCompleted: if (player.source != "") player.play()
}
