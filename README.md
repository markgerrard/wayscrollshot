# wayscrollshot

An Omarchy / Hyprland fork of [jswysnemc/wayscrollshot](https://github.com/jswysnemc/wayscrollshot), with automatic scrolling, safer stitching, and a native capture UI.

## This fork

The optional [Omarchy panel integration](integrations/omarchy/README.md) adds a capture icon and dropdown menu to the top panel, including the local cloud gallery.

**Super + Shift + Print Screen** opens the unified capture bar near the top of the screen on this Omarchy installation. Area captures on drag release, Full screen captures when selected, and Window captures when clicked. Scrolling collapses the mode bar to **Auto-scroll** and **Start capture** pills. Start capture is manual; Auto-scroll starts with page movement enabled.

From the terminal, use `wayscrollshot --capture-bar` for the mode bar or `wayscrollshot --screenshot` for a single screenshot. The original scrolling shortcuts remain available.

- Drag to select, press **Space** to select the window under the pointer, or use **F / Full screen** for the monitor where selection started.
- In scrolling mode, adjust any edge or corner before choosing **Start capture** or **Auto-scroll**. In browsers, lower the top edge to exclude tabs and the address bar.
- Dimmed capture mask, a bounded live preview that follows new content, and labeled controls with ImageGen artwork.
- Automatic scrolling advances relative to crop height and learns the application's scroll distance. Uncertain overlaps retry after settling, then backtrack with smaller jumps.
- Moving the pointer outside the capture area pauses automatic scrolling; returning resumes it. Finishing remains available while paused.
- The preview stays outside the capture. When no space is available (including full screen), it is omitted during capture and shown for final review. Run `wayscrollshot --toggle` to finish.

The native selector and automatic scrolling use `hyprctl`; explicit geometry remains available for other compatible Wayland compositors. Only the monitor under the pointer when selection opens is used by the native selector.

On this Omarchy setup, **Super + R** starts/finishes a manual scrolling capture and **Super + Alt + R** opens the manual scrolling selector. Choose **Auto** in the live card to switch the running capture to automatic scrolling. These shortcuts are local configuration, not installed by the build.

The screenshots and distribution instructions below originated upstream; prebuilt upstream releases do not contain these fork changes.
![preview](./preview.png)

[中文文档](README_CN.md)

## Features

- Real-time preview with automatic stitching
- Column sampling algorithm for fast and accurate overlap detection
- Rounded button UI with hover effects (powered by tiny-skia)
- Keyboard shortcuts and mouse control
- Save to file or copy to clipboard
- Upload to DigitalOcean Spaces, copy a CDN link, and browse recent cloud captures
- Supports reverse scrolling (col-sample only)

## How It Works

### Architecture

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   Capture   │────>│   Stitcher   │────>│   Preview   │
│   (grim)    │     │ (col-sample) │     │ (layer-shell)│
└─────────────┘     └──────────────┘     └─────────────┘
```

### Column Sampling Algorithm

Instead of comparing entire images pixel-by-pixel, wayscrollshot uses a column sampling approach inspired by [screenshot-splicing](https://github.com/baotlake/screenshot-splicing):

1. **Sample 3 column groups** from each frame:
   - Left region (20 to width/4)
   - Middle region (width/2 to 5\*width/8)
   - Right region (6\*width/8 to 7\*width/8)

2. **Convert to grayscale** and average each group

3. **Search for overlap** using Mean Absolute Difference (MAD):
   - Start from the predicted offset (based on previous scroll)
   - Expand search outward: `[p, p+1, p-1, p+2, p-2, ...]`
   - Early termination when MAD < threshold

4. **Append new content** to the stitched image

**Complexity**: O(9 \* height) instead of O(width \* height) - a significant speedup.

### Overlap Detection

```
Frame 1 (previous):          Frame 2 (current):
┌────────────────┐           ┌────────────────┐
│    Content A   │           │    Content B   │
│                │           │                │
│    Content B   │ <──────── │    Content B   │  (overlap)
│                │           │                │
│    Content C   │           │    Content C   │
└────────────────┘           │                │
                             │    Content D   │  (new)
                             └────────────────┘
```

The algorithm finds where Frame 2's top matches Frame 1's content, then appends only the new portion.

## Dependencies

### Runtime Dependencies

| Tool / Library | Purpose | Required |
|----------------|---------|----------|
| Hyprland / `hyprctl` | Native region/window selection and automatic scrolling | Unless using explicit geometry with manual scrolling |
| Fontconfig (`fc-match`) and a sans-serif font | UI text | Yes |
| `slurp` | Optional external region selection | No |
| `grim` | Screen capture | Yes |
| OpenCV | Optional stitching algorithms linked at build time | Yes |
| `wl-copy` | Clipboard (Wayland) | For clipboard feature |
| `xclip` | Clipboard (X11 fallback) | Alternative |

### Build Dependencies

| Crate | Purpose |
|-------|---------|
| `smithay-client-toolkit` | Wayland client library |
| `wayland-client` | Wayland protocol bindings |
| `tiny-skia` | 2D graphics (rounded buttons) |
| `image` | Image processing and resizing |
| `opencv` | ORB feature matching and RANSAC alignment |
| `clap` | Command-line argument parsing |
| `anyhow` | Error handling |
| `chrono` | Timestamp for filenames |
| `log` / `env_logger` | Logging |

OpenCV development files and Clang are required when building from source.

## Installation

### Debian / Ubuntu

Download the `.deb` package that matches your Ubuntu release and CPU architecture from GitHub Releases, then install it with `apt`:

```bash
sudo apt install ./wayscrollshot_0.1.7-1ubuntu26.04_amd64.deb
```

The Debian packages are built separately for each Ubuntu release so OpenCV runtime dependencies match that release's ABI. Do not work around missing OpenCV libraries by symlinking a different OpenCV version.

### From Source

```bash
# Install runtime and build dependencies (Arch Linux)
sudo pacman -S grim wl-clipboard opencv clang libxkbcommon fontconfig ttf-liberation

# Build
cargo build --release

# Install (optional)
cp target/release/wayscrollshot ~/.local/bin/
```

### From Nix

#### Run

```shell
nix run github:jswysnemc/wayscrollshot
```

#### home-manager configuration

```nix
# input
inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  
  wayscrollshot = {
    url = "github:jswysnemc/wayscrollshot";
    inputs.nixpkgs.follows = "nixpkgs";
  };
}
```

## Usage

```bash
# Basic usage
wayscrollshot

# Save to specific file
wayscrollshot -o ~/screenshot.png

# Copy to clipboard instead of saving
wayscrollshot -c

# Custom preview width
wayscrollshot -w 320

# Disable preview window
wayscrollshot --no-preview

# Browse captures uploaded from this machine
wayscrollshot --cloud-gallery

# Disable region border overlay
wayscrollshot --no-border

# Use an existing slurp selection
wayscrollshot "$(slurp)"

# Read slurp output from stdin
slurp | wayscrollshot -

# Use different stitching algorithms
wayscrollshot -a opencv-orb  # ORB feature matching + RANSAC
wayscrollshot -a col-sample  # Default: verified column sampling
wayscrollshot -a template    # Template matching (more accurate)
wayscrollshot -a edge        # Edge detection (for transparent backgrounds)
wayscrollshot -a fast        # FAST corner + HNSW index (experimental)
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output <PATH>` | Output file path | `~/Pictures/wayscrollshot/wayscrollshot-<timestamp>.png` |
| `-w, --preview-width <PX>` | Preview width in pixels | 280 |
| `-c, --clipboard` | Copy to clipboard instead of saving | false |
| `--no-preview` | Disable preview window | false |
| `--no-border` | Disable region border overlay | false |
| `--cloud-gallery` | Open the cached cloud-capture gallery | false |
| `-a, --algorithm <ALG>` | Stitching algorithm: `opencv-orb`, `col-sample`, `template`, `edge`, `fast` | col-sample |
| `REGION` | Existing slurp/grim geometry, for example `10,20 300x400`; use `-` to read stdin | Native selector |

### Controls

During selection, Area captures when the drag is released, Window captures when the target is clicked, and Full screen captures as soon as it is selected. Scrolling first shows a selection prompt with no toolbar: drag an area or press Space to select a window. After selection, Auto-scroll and Start capture appear above the region. Enter starts manual capture. Esc cancels. A scrolling region must be at least 32 × 160 pixels.

```bash
wayscrollshot --auto-scroll
wayscrollshot --toggle  # finish the running capture (or start if none exists)
```


**Mouse:**
- Click buttons in the control bar

**Control bar buttons:**
| Button | Action |
|--------|--------|
| Done / Save | Finish, save image and exit |
| Copy | Copy to clipboard and exit |
| Cloud (final review) | Save locally, upload to Spaces, and copy the CDN link |
| Auto (manual scrolling) | Switch the running capture to automatic scrolling |
| Pause / Resume (live only) | Pause or resume capture |
| Cancel | Cancel capture and exit |

**Keyboard (when overlay is focused):**
| Key | Action |
|-----|--------|
| `S` | Save and exit |
| `C` | Copy to clipboard and exit |
| `U` | Upload to cloud and copy its link from final review |
| `A` | Switch a manual scrolling capture to Auto |
| `Space` | Pause/Resume capture |
| `Q` / `Esc` | Cancel and exit |

> **Note:** Keyboard shortcuts depend on the compositor's `wlr-layer-shell` `OnDemand` keyboard focus policy. They work well under niri; on Hyprland and some other compositors you may need to hover the mouse over the control bar first. Mouse clicks on the control bar buttons work on all compositors.

### Cloud gallery

The Cloud action reads DigitalOcean Spaces settings from `~/.config/wayscrollshot/cloud.toml` and credentials from the desktop Secret Service keyring. It always saves the original capture locally before uploading it. The CDN link is copied to the clipboard after a successful upload.

The Omarchy panel expands into a centered carousel of cached cloud captures. Choosing a card restores that image to the normal review preview with **Save**, **Copy**, **Cloud**, and **Cancel**. `wayscrollshot --cloud-gallery` restores the newest cached capture directly; pass `--cloud-id` to restore a specific one. SQLite metadata lives at `~/.cache/wayscrollshot/cloud.sqlite3`. Small preview images are cached for `cache_ttl_hours` (24 hours by default); expiring a preview does not delete the local screenshot or its Spaces object.

## Limitations

1. **Wayland only**: X11 is not supported. The tool uses `wlr-layer-shell-unstable-v1` protocol.

2. **wlroots-based compositors**: Works on Sway, Hyprland, river, etc. May not work on GNOME/KDE Wayland.

3. **Overlap requirement**: Each scroll step must leave some overlap with the previous view. Very fast scrolling may cause stitching failures.

4. **Static content assumption**: The algorithm assumes the scrolling content is static. Dynamic content (animations, videos) will cause artifacts.

5. **Vertical scrolling only**: Horizontal scrolling is not currently supported.

6. **Fixed header/footer**: Overlap joins reduce repeated sticky content. Selecting just the scrolling content is still preferable, particularly with large toolbars or animation.

## Troubleshooting

### Native selector cannot read desktop geometry
- Ensure you are running Hyprland and `hyprctl` is available.
- On other layer-shell compositors, pass explicit geometry or use `wayscrollshot "$(slurp)"`.

### "layer-shell not available"
- Your compositor doesn't support `wlr-layer-shell-unstable-v1`
- Try a wlroots-based compositor (Sway, Hyprland)

### "No overlap match"
- Scroll more slowly
- Ensure there's visible overlap between frames
- Avoid scrolling through completely different content

### Preview not updating
- Check if the capture region is correct
- Try running with `RUST_LOG=debug` for more info

## License

MIT

## Contributing

Contributions are welcome! Areas that could use improvement:

- **Algorithm optimization**: The `fast` algorithm (FAST corner + HNSW) needs tuning for better accuracy
- **Cross-platform support**: Currently Linux-only due to Wayland dependency
- **Performance**: Reduce memory usage for very long screenshots
- **UI improvements**: Better visual feedback during capture

Please open an issue to discuss major changes before submitting a PR.

## Acknowledgments

- [screenshot-splicing](https://github.com/baotlake/screenshot-splicing) - Column sampling algorithm inspiration
- [snow-shot](https://github.com/mg-chao/snow-shot) - FAST corner + HNSW algorithm reference
- [smithay-client-toolkit](https://github.com/Smithay/client-toolkit) - Wayland client library
- [tiny-skia](https://github.com/RazrFalcon/tiny-skia) - 2D graphics library

- Friend Links[Linux.do社区](https://linux.do/)
