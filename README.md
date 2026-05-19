# Player Sage

A focused Windows video player built for inspecting frames closely. Powered by libmpv — handles every codec/container mpv handles, including ProRes, HEVC variants, and oddball MOVs that the system player chokes on.

## Features

- Plays MOV, MP4, M4V, MKV, WebM, AVI (anything libmpv supports)
- **Frame-accurate zoom & pan** anchored at the cursor — built for inspection
- **A/B looping** with single-key set/clear
- **Variable playback speed** down to ⅛ × (no fast-forward — Player Sage is for studying frames, not skimming)
- **Drag-and-drop** video files onto the window (or anywhere)
- Auto-update from GitHub Releases

## Keyboard

| Key                    | Action                                                |
|------------------------|-------------------------------------------------------|
| `Space` / click video  | Play / pause                                          |
| `←` / `→`              | Seek ±5 s                                             |
| `Shift+←` / `Shift+→`  | Seek ±20 s                                            |
| `Ctrl+←` / `Ctrl+→`    | Seek ±1 s                                             |
| `A`                    | Set loop start at current time                        |
| `B`                    | Set loop end at current time (loop activates)         |
| `C`                    | Clear loop                                            |
| `R` or `0`             | Reset zoom & pan                                      |

## Mouse

- **Wheel** over video: zoom, anchored at cursor (`Ctrl+wheel` for finer steps)
- **Right-drag** over video: pan
- **Double-click** zoom % button: reset zoom & pan
- **Drag a file** onto the window: load and play

## Stack

- Tauri 2 (Rust) + Vite + React 19 + TypeScript + Tailwind 4
- Video engine: libmpv via the `libmpv2` crate
- Rendered into a native child HWND parented to the Tauri window; controls live in a chrome strip outside the video area (no overlays on the frame)

## Building from source

Prereqs:
- Node 20+ (Vite 7 requires it)
- Rust + Cargo
- Visual Studio 2022 Build Tools (C++ workload) — needed for `lib.exe` / `dumpbin.exe`
- 7-Zip (`C:\Program Files\7-Zip\7z.exe`)

```powershell
# 1. Fetch libmpv (downloads ~30 MB, ~one-time)
powershell -ExecutionPolicy Bypass -File .\scripts\setup-libmpv.ps1

# 2. JS deps
npm install

# 3. Run
npm run tauri dev
```

To refresh libmpv to the latest shinchiro build:
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\setup-libmpv.ps1 -Force
```

## License

Personal project by Robert Mirabelle. No license granted at this time.
