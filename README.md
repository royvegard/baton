# Baton
Mixer control application for PreSonus STUDIO1824c audio interface.
Written in Rust using Ratatui for terminal user interface.

## Features
- 9 stereo mixes for all 36 input channels.
- Solo, mute, and bypass for input channels.
- Pan/balance control for all channel strips.
- Mute for the 9 stereo mixes.
- Metering with adjustable height.
- MIDI control support:
  - ALSA MIDI sequencer port for receiving MIDI control messages.
  - Configurable MIDI CC mapping to mixer controls (fader, balance, mute, solo).
  - MIDI learn mode for easy mapping.
  - Persistent MIDI mapping configuration.
- Strip renaming.
- Adjustable strip width.
- Toggle buttons for:
  - 48V phantom power.
  - Input 1-2 Line level signal.
  - Main output mute.
  - Main output mono mode.

## Key mapping
| Function | Key |
|----------|-----|
| Select strip | Left/Right arrow |
| Select mix | 1-9 |
| Change volume 1.0 dB | Up/Down arrow |
| Change volume 10.0 dB | Ctrl + Up/Down arrow |
| Change volume 0.1 dB | Shift + Up/Down arrow |
| Change strip width | Ctrl + Left/Right arrow |
| Change meter height | PgUp/PgDn |
| Pan left 1.0 | x |
| Pan left 10.0 | Ctrl + x |
| Center pan | c |
| Pan right 1.0 | v |
| Pan right 10.0 | Ctrl + v |
| Solo | s |
| Mute | m |
| Bypass | b |
| Clear clip indicators | Space |
| Toggle 48V phantom power | p |
| Toggle 1-2 line input mode | l |
| Toggle Main output mute | u |
| Toggle Main output mono | o |
| MIDI Learn - Fader | Shift + F |
| MIDI Learn - Balance | Shift + B |
| MIDI Learn - Mute | Shift + M |
| MIDI Learn - Solo | Shift + S |
| Cancel MIDI Learn | Esc |
| Rename strip | r |
| Enter command mode | : |
| Quit | q |

## Using Baton

```
                      +----------+     +------------+
                      | Out 1,2  | ... | Out 17,18  |
                      +----------+     +------------+
                        0|  1|           0|  1|
+------------+ 0         |   |            |   |
|           =|-----------+   |            |   |
| I     Mix 1| 1         |   |            |   |
| n      .  =|-----------|---+            |   |
| p      .   |           |   |            |   |
| u      .   | 0         |   |            |   |
| t         =|-----------|---|------------+   |
|       Mix 9| 1         |   |            |   |
| 1         =|-----------|---|------------|---+
|            |           |   |            |   |
+------------+           |   |            |   |
     ...                 |   |            |   |
     ...                 |   |            |   |
     ...                 |   |            |   |
+------------+ 0         |   |            |   |
|           =|-----------+   |            |   |
| I     Mix 1| 1         |   |            |   |
| n      .  =|-----------|---+            |   |
| p      .   |           |   |            |   |
| u      .   | 0         |   |            |   |
| t         =|-----------|---|------------+   |
|       Mix 9| 1         |   |            |   |
| 36        =|-----------|---|------------|---+
|            |           |   |            |   |
+------------+           |   |            |   |

```
