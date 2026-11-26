# Icon assets

Both files come from `AppIcon.svg` (1024 px, the source of truth).

| File | Frames | Consumer |
| --- | --- | --- |
| `app_icon.png` | 88×88 | `studio.rs` `APP_ICON_PNG` — 18 px title-bar mark, settings, dialogs |
| `AppIcon.ico` | 16, 24, 32, 48, 256 | `build.rs` resource ID 1 for the exe; `installer/larik-sql-studio.iss` `SetupIconFile` |

## Regenerate

Render each frame at its own size — downscaling one large bitmap makes the
16 px frame soft.

```powershell
$svg = 'crates/larik-ui/assets/AppIcon.svg'
$dir = 'crates/larik-ui/assets'

magick -background none $svg -depth 8 -resize 88x88 `
       -define png:exclude-chunks=date,time PNG32:"$dir/app_icon.png"

foreach ($s in 16,24,32,48,256) {
  magick -background none $svg -depth 8 -resize "${s}x${s}" `
         -define png:exclude-chunks=date,time PNG32:"$dir/_frame-$s.png"
}
# pack PNG frames into a 32-bit ICO; ImageMagick's own ICO writer emits one
# oversized frame that Explorer scales badly at 16 px
python "$dir/pack_ico.py"
Remove-Item "$dir/_frame-*.png"
```

Two flags matter:

- `-depth 8` — ImageMagick defaults to 16-bit PNGs, which some shell surfaces
  render with a colour shift.
- `-define png:exclude-chunks=date,time` — without it every run rewrites the
  `tIME`/`date:*` chunks, so identical art produces a dirty working tree and
  an unreviewable binary diff.

## Palette

Emerald ramp, matched stop-for-stop to the lightness of the indigo original so
the white glyph keeps identical contrast:

| Stop | Emerald |
| --- | --- |
| 0 % | `#089B6A` |
| 55 % | `#078A5B` |
| 100 % | `#046743` |
| glyph / rows | `#F7F7FB` |

See `docs/adr/0009-green-brand-accent-and-icon-variant.md` for how the UI accent
relates to this tile colour.
