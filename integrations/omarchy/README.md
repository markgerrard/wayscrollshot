# Omarchy top-panel menu

Copy `mark.capture/Panel.qml` and `manifest.json` into `~/.config/omarchy/plugins/mark.capture/`, together with `assets/capture-icons.png` and `assets/capture-modes.png` from the repository root. Add `{"id":"mark.capture"}` to the desired bar layout section in `~/.config/omarchy/shell.json` after backing up the file, then run `omarchy-shell shell rescanPlugins`.

The menu runs `~/.local/bin/wayscrollshot`, so install the current release build first. It offers All-in-One, Area, Fullscreen, Window, and automatic Scrolling capture. It closes before launching a capture, keeping the menu out of the image. Keyboard navigation and Escape use Omarchy's native panel controls.
