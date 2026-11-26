# Icon sources

- `icon.svg`: the app icon (a flint stone with a spark in front).
- `tray.svg`: the menu bar icon, a macOS template image (only its alpha is used).

Regenerate the files in `icons/` (needs `rsvg-convert`, from librsvg):

```shell
rsvg-convert -w 1024 -h 1024 icon.svg -o /tmp/flintkey-icon.png
pnpm -C apps/desktop tauri icon /tmp/flintkey-icon.png -o /tmp/flintkey-icons
cp /tmp/flintkey-icons/{32x32.png,128x128.png,128x128@2x.png,icon.icns,icon.ico} apps/desktop/src-tauri/icons/
rsvg-convert -w 44 -h 44 tray.svg -o apps/desktop/src-tauri/icons/tray-template.png
```
