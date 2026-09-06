# OpenCrate branding

Original OpenCrate artwork created with ImageGen. Assets use an opaque charcoal
background with an amber emblem. The square master is resized for runtime icons.
No ASUS artwork is included. Image metadata is removed before publication.

## Files

- `opencrate-logo.png`: horizontal amber emblem and OpenCrate wordmark, 1905 Ã— 825.
- `opencrate-icon.png`: generated square icon master, 1254 Ã— 1254.
- `opencrate-icon-256.png`: application window/taskbar icon.
- `opencrate-icon-48.png`: notification area icon.
- `opencrate-icon.ico`: Windows executable icon with 16, 24, 32, 48, 64, 128 and 256 px entries.

The PNG icons are embedded by opencrate-ui. Its build script embeds the ICO as a Windows resource via embed-resource, with no privilege/manifest changes.

## Regenerating runtime sizes

```powershell
magick assets/branding/opencrate-icon.png -strip -resize 256x256 assets/branding/opencrate-icon-256.png
magick assets/branding/opencrate-icon.png -strip -resize 48x48 assets/branding/opencrate-icon-48.png
magick assets/branding/opencrate-icon.png -strip -define icon:auto-resize=256,128,64,48,32,24,16 assets/branding/opencrate-icon.ico
```
