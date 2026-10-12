# Mira brand assets

Original artwork supplied for Mira branding.

- `mira_logo_new.png`: app-icon artwork.
- `harness-icons/mira_harness_color.png`: transparent mark used by the UI and landing site.
- `harness-icons/`: complete colour, monochrome, 3D and motion variants.
- `mira_harness_icon_pack.zip`: original archive, retained for future use.
- `Mira_macOSInstaller.png`: original installer artwork.

Native icons are generated with `cargo tauri icon`, then `python3 scripts/channel-icons.py` adds the alpha/beta badges. Run `python3 scripts/installer-background.py` on macOS to prepare the DMG background at its configured logical size.
