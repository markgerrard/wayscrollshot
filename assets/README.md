# Capture UI artwork

`capture-icons.png` is original ImageGen artwork created for this fork on 2026-09-12. The six symbols are capture, copy, pause, resume, cancel and full screen. The transparent source sheet is embedded in the binary; `src/ui.rs` selects each symbol and caches its 20-pixel rendering. Button shapes and labels are rendered natively for readable text and accurate hit targets.
