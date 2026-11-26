# ADR 0009: Green Brand Accent, Icon Variant & Status-Hue Separation

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-ui/src/theme.rs`, `crates/larik-ui/src/studio.rs`, `crates/larik-ui/src/mcp_page.rs`, `crates/larik-ui/assets`

## Context

The GPUI app shipped with the Windows 11 Fluent system accent (`#60CDFF` dark / `#005FB8` light). That accent is indistinguishable from any other Fluent app, and the app icon was still the inherited indigo tile from the WinUI port, so the shell and the taskbar icon disagreed.

The green direction was already chosen for the product icon (an L bent from one bar with three fading rows), so the accent needed to move to the same family rather than stay system blue.

## Decisions

### 1. Accent is Larik emerald, not the Fluent system accent
| Token | Dark | Light |
|---|---|---|
| `accent` | `#2FD3A0` | `#047857` |
| `accent_hover` | `#45DFB0` | `#05805E` |
| `accent_on` | `#04231A` | `#FFFFFF` |

Each stop is matched to the contrast of the Fluent accent it replaces, so text and glyph legibility on primary buttons is unchanged:

- Dark accent on the `#161618` shell: **9.4:1** (was 10.0:1 for `#60CDFF`); dark ink `#04231A` on the accent: **8.7:1** (was 10.2:1).
- Light accent on white: **5.5:1** (was 6.3:1 for `#005FB8`); white on the accent: **5.5:1** (was 6.3:1). Both clear WCAG AA for normal text and AAA for large text.

`crates/larik-ui/src/theme.rs` asserts these ratios in `brand_accent_matches_the_icon_ramp_and_stays_legible`, so a future palette edit fails the test rather than silently degrading legibility.

`parse_hex_color` falls back to the dark accent instead of Fluent blue, so an unparseable workspace colour no longer reintroduces a blue.

### 2. Soft accent tints derive from the accent hue, not a literal channel mix
`accent_soft`, `selection`, `tab_active_bg` and the workspace-flyout pill all moved from hues 199–214 to 161–163. Their lightness and alpha are unchanged from the Fluent set, so the surfaces keep the same weight; only the hue rotates. Because the dark tints are low-alpha over Mica, saturation had to rise (0.28→0.50 for `tab_active_bg`, 0.65→0.80 for `accent_soft`, 0.24→0.40 for the flyout pill) to stay visibly green rather than reading as neutral slate.

### 3. `success` shifts to a yellow-leaning green
The old dark `success` `#34D399` sat 1.7 ΔE2000 from the new accent, which would have made "connected" and "primary action" the same colour. Success moved to `#22C55E` dark (**11.1 ΔE** from the accent) and stays `#15803D` light (**8.7 ΔE**). `accent_and_success_share_a_hue_but_stay_distinguishable` enforces a minimum 8 ΔE in both modes.

Status is still communicated by text plus colour, so the shared hue family is safe: the wording ("Connected", "Failed") carries the meaning and the hue only reinforces it. This deliberately keeps a single green family rather than splitting the product into two greens.

### 4. Icon assets ship at the sizes the binary consumes
`AppIcon.ico` carries 16/24/32/48/256 PNG frames and `app_icon.png` is 88×88. `build.rs` embeds the `.ico` at Win32 resource ID 1, and `studio.rs` `include_bytes!`s the PNG for the 18 px title-bar mark, so both must be the green variant or the taskbar and the in-app mark disagree. The installer reuses the same `.ico` (`SetupIconFile`).

The frames are packed as 32-bit PNGs; ImageMagick's ICO writer emits a single oversized frame that Explorer scales poorly at 16 px.

## Consequences

- The shell, the taskbar icon and the installer now share one brand colour.
- Any future accent change must re-check the `success`/`accent` ΔE distance, not just the two colours in isolation.
- Light mode keeps white primary-button text, so `accent_on` stays white there while dark mode uses dark ink; a single shared `accent_on` would fail contrast in one of the two modes.
