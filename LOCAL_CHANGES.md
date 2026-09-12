# Scrolling capture fixes

This local wayscrollshot branch is the scrolling-capture prototype alongside OpenShotX.

## Controls

- `wayscrollshot --toggle`: start capture, or finish the current capture.
- `wayscrollshot --toggle --auto-scroll`: automatic downward scrolling in Hyprland.
- During capture no overlay or border is mapped. Finish with the shortcut again.
- A review panel appears after capture has stopped: Save, Copy, or Cancel.
- `--no-preview`: save immediately (useful for testing).
- `--settle-ms 450`: minimum stable interval; can increase for slow pages.
- Moving the pointer away from the capture center stops automatic scrolling.

## Changes

Preserve the accepted reference frame on rejected matches and reverse scrolling.
Require sampled RGB overlap agreement and nonblank texture before appending.
Ignore the top/bottom 15% when computing column matches and validating overlaps.
Join within overlap, replacing the previous frame's bottom portion.
Wait for stable pixels before accepting frames. Auto-scroll advances one wheel tick
at a time, stops after four unchanged samples, and stops on uncertain matches.
A runtime socket provides start/finish control without photographing controls.
Auto-scroll has a three-minute and 192 MiB output limit. Unstable or uncertain pages
produce a partial capture and a completion explanation rather than guessed joins.

## Limits

Automatic pointer positioning currently requires Hyprland. Continuous video,
large sticky regions, parallax, or infinite scrolling can prevent a complete capture.
Manual mode should be scrolled in small steps with pauses between them.
This captures rendered pixels, not the browser's underlying document.

## Validation

Unit regression cases cover rejected frames, reverse scrolling, misleading column
averages, blank overlaps, animation settling, and fixed header/footer joins.
A live Chrome page with 18 numbered sections was auto-scrolled and stitched to a
920 x 2873 image, reaching the page bottom without repeated sections.

Installed controls: Super+R starts/finishes manual capture; Super+Alt+R starts auto-scroll.
Validation completed: 24 tests passed; release build succeeded; clippy succeeded
with seven pre-existing style warnings. Installed shortcut IPC smoke test passed.
