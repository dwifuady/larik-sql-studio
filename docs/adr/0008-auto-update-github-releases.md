# 0008. GitHub Releases Self-Update: Asset Selection, Verification & Install Hand-Off

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `larik-update`, `larik-ui`, `larik-core`, `installer`, `.github/workflows`

## Context

Larik SQL Studio ships from two places: an Inno Setup per-user installer and a portable ZIP. Users on either shape expect to learn about a newer build without watching GitHub, and the app has to replace its own files. That replacement is the risky part: the app is running while it overwrites itself.

The WinUI edition of Larik already solved this in `UpdateService`; this ADR records the same contract for the Rust/GPUI studio and the parts that differ.

## Decisions

### The feed is `releases/latest`, and drafts/prereleases are ignored

The check reads `https://api.github.com/repos/dwifuady/larik-sql-studio/releases/latest` with the GitHub JSON accept header and a 15-second budget. A `404` means no release has been published yet and is treated as up to date rather than as an error. Draft and prerelease records are also up to date. The release workflow uploads a **draft** with `generate_release_notes`, so a check that ran against an unpublished draft would otherwise offer an installer that does not exist yet.

### Asset selection keys off install shape and architecture

`InstallKind` is decided once at startup by looking for `unins000.exe` in the app folder, then by matching the Inno `InstallLocation` registry value (`{A7C4F8E2-…}_is1`) against the running folder. Anything else is a portable copy. The loader then requires:

- installer → `Larik-SQL-Studio-v{version}-windows-{arch}-setup.exe`
- portable → `Larik-SQL-Studio-v{version}-windows-{arch}-portable.zip`

`{arch}` comes from `std::env::consts::ARCH` limited to `x64`/`arm64`; anything else opts out. Installing a per-machine setup over a portable folder (or a ZIP over an installed copy) is exactly the kind of mismatch that leaves two half-broken copies on disk, so the shape of the running copy picks the asset and nothing else does.

### Verification fails closed on the published `SHA256SUMS.txt`

Every release publishes `SHA256SUMS.txt`. A release without it, an unreachable checksum URL, a missing entry for the selected asset, or a digest mismatch all block the install. TLS alone is not evidence: it only says the bytes came from GitHub, not that the release was the one the maintainer built. The digest is re-checked at install time, not only at download time, so a file edited between download and install is still refused.

### Downloads come only from GitHub over HTTPS

`is_trusted_download_url` accepts `https` on `github.com`, `*.github.com`, `githubusercontent.com`, `*.githubusercontent.com`, and rejects any URL carrying userinfo (`https://github.com@evil…`). The API's `browser_download_url` normally points at `github.com` and redirects to `objects.githubusercontent.com`; those two hosts cover the real flow without accepting an arbitrary download origin that the release record could be tricked into naming.

### The app writes its own replacement and then exits

The `larik-update` crate is deliberately UI-free and dependency-light (`ureq` + `zip` + `sha2`). Two hand-offs exist because the two install shapes differ in who can write:

- **Installer:** start `…-setup.exe` with `/SILENT /NORESTART /CLOSEAPPLICATIONS /NORESTARTAPPLICATIONS /RELAUNCH=1 /DIR="<running folder>" /LOG=…`. `/DIR` keeps a moved install in the folder the user is actually running instead of falling back to the default path; `/RELAUNCH=1` reopens Larik once files are in place. The installer sets `CloseApplications=yes` so it can wait for this process.
- **Portable:** unpack the ZIP to `staging/` under the app data `updates/` folder, then start a hidden `cmd.exe` that polls `tasklist` for this PID, `robocopy`s the staging tree over the running folder, and relaunches `larik-sql-studio.exe`. Paths travel in environment variables (`LARIK_UPDATE_PID/SOURCE/TARGET/EXE`) so a profile with non-ASCII characters survives the batch file's OEM code page; the script itself stays ASCII.

In both cases the app calls `cx.quit()` immediately after the helper is started. Anything the app still holds — unsaved notes, window placement, the snapshot — is flushed first through `flush_before_update_exit`. A portable copy whose folder is not writable is rejected **before** the app closes, so a permissions failure is a message rather than a vanished window.

### Debug builds never replace themselves

`can_self_update()` is true only on Windows release builds. A `cargo run` or a debug CI artifact would otherwise download a release ZIP and copy it over `target/debug`, destroying the build under test. `LARIK_ALLOW_SELF_UPDATE=1` opts a debug build in for manual testing.

### The launch check is once per launch, delayed, and silent on failure

A check runs 1.2 s after the window opens, only when the user has not turned `check_for_updates` off (persisted in `StudioState`, default on, following the WinUI edition's toggle). Network, rate-limit, and parse failures never open a dialog: they update the status line in Settings and let the next launch try again. Only an available update, a download in progress, a ready-to-install build, and a failed *manual* check surface the floating notice, which is dismissible and never blocks the workspace.

The Settings → Updates page exposes the same state and a manual **Check now** that reports `Available`/`UpToDate`/`Failed` inline, matching the WinUI `CheckNowAsync` behaviour.

## Consequences

- A new release must include all three assets the updater knows about (`setup.exe`, `portable.zip`, `SHA256SUMS.txt`). The release workflow already builds them; the checksum file is computed from every file in `dist/`, so it covers both executables.
- Renaming an asset is a breaking change for in-flight updaters; the fallback matcher (suffix + version substring) tolerates extra decorations but not a changed suffix.
- Self-update is unavailable to any future MSIX/Store build. A packaged copy has read-only files and belongs to the package owner; the feature must stay off there rather than offering an installer that cannot write.
- `larik-update` starts a process and then exits, so an interrupted update leaves the downloaded file under the app data `updates/` folder. The next check reuses a complete, verified file and deletes stale versions.
