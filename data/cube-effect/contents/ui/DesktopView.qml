/*
    SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
    Based on the KDE Cube effect (kdeplasma-addons), SPDX-FileCopyrightText: 2022 Vlad Zahorodnii
*/

import QtQuick
import org.kde.kwin as KWinComponents

// Everything on one desktop of one screen, laid out like the screen (it is the picture drawn on a cube face).
Item {
    id: desktopView

    required property QtObject desktop
    required property QtObject targetScreen

    Repeater {
        model: KWinComponents.WindowFilterModel {
            activity: KWinComponents.Workspace.currentActivity
            desktop: desktopView.desktop
            screenName: desktopView.targetScreen.name
            windowModel: KWinComponents.WindowModel {}
        }

        KWinComponents.WindowThumbnail {
            wId: model.window.internalId
            x: model.window.x - desktopView.targetScreen.geometry.x
            y: model.window.y - desktopView.targetScreen.geometry.y
            z: model.window.stackingOrder
            visible: !model.window.minimized
        }
    }
}
