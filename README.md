# Baton
Mixer control application for PreSonus STUDIO1824c audio interface.
Written in Rust using Ratatui for terminal user interface.

## Features
- 8 stereo mixes for all 36 input channels.
- Solo and mute for input channels.
- Mute for the 8 stereo mixes.
- Toggle buttons for:
  - 48V phantom power.
  - Input 1-2 Line level signal.
  - Main output mute.
  - Main output mono mode.

## Key mapping
| Function | Key |
|----------|-----|
| Select strip | Left/Right arrow |
| Select mix | 1-8 |
| Change volume 1.0 dB | Up/Down arrow |
| Change volume 10.0 dB | Ctrl + Up/Down arrow |
| Change volume 0.1 dB | Shift + Up/Down arrow |
| Change strip width | Ctrl + Left/Right arrow |
| Pan left | x |
| Center pan | c |
| Pan right | v |
| Solo | s |
| Mute | m |
| Toggle 48V phantom power | p |
| Toggle 1-2 line input mode | l |
| Toggle Main output mute | u |
| Toggle Main output mono | o |
| Quit | q |

