use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const REPOSITORY: &str = "dwifuady/larik-sql-studio";
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS.txt";
pub const APP_EXE: &str = "larik-sql-studio.exe";

const RELEASES_API: &str = "https://api.github.com/repos/dwifuady/larik-sql-studio/releases/latest";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Installer,
    Portable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    pub checksums_url: String,
    pub install_kind: InstallKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Unavailable,
    UpToDate,
    Available,
    Failed,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub status: CheckStatus,
    pub update: Option<UpdateInfo>,
    pub error: Option<String>,
}

impl CheckResult {
    fn unavailable() -> Self {
        Self {
            status: CheckStatus::Unavailable,
            update: None,
            error: None,
        }
    }

    fn up_to_date() -> Self {
        Self {
            status: CheckStatus::UpToDate,
            update: None,
            error: None,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Failed,
            update: None,
            error: Some(message.into()),
        }
    }

    fn available(info: UpdateInfo) -> Self {
        Self {
            status: CheckStatus::Available,
            update: Some(info),
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub received: u64,
    pub total: Option<u64>,
}

impl DownloadProgress {
    pub fn percent(&self) -> Option<f64> {
        match self.total {
            Some(total) if total > 0 => {
                Some((self.received as f64 * 100.0 / total as f64).clamp(0.0, 100.0))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    Network(String),
    Rejected(u16),
    UnsafeUrl(String),
    Cancelled,
    Io(String),
    Integrity(String),
    Unavailable(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Network(message) => write!(f, "{message}"),
            UpdateError::Rejected(code) => write!(f, "GitHub answered {code}"),
            UpdateError::UnsafeUrl(url) => write!(f, "Refused to download from {url}"),
            UpdateError::Cancelled => write!(f, "The update was cancelled"),
            UpdateError::Io(message) => write!(f, "{message}"),
            UpdateError::Integrity(message) => write!(f, "{message}"),
            UpdateError::Unavailable(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<std::io::Error> for UpdateError {
    fn from(error: std::io::Error) -> Self {
        UpdateError::Io(error.to_string())
    }
}

impl From<ureq::Error> for UpdateError {
    fn from(error: ureq::Error) -> Self {
        match error {
            ureq::Error::StatusCode(code) => UpdateError::Rejected(code),
            other => UpdateError::Network(other.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct UpdateService {
    agent: ureq::Agent,
    updates_root: PathBuf,
    current_version: String,
    install_kind: InstallKind,
}

impl UpdateService {
    pub fn new(updates_root: PathBuf) -> Self {
        Self {
            agent: build_agent(),
            updates_root,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            install_kind: detect_install_kind(),
        }
    }

    pub fn current_version(&self) -> &str {
        &self.current_version
    }

    pub fn install_kind(&self) -> InstallKind {
        self.install_kind
    }

    pub fn updates_root(&self) -> &Path {
        &self.updates_root
    }

    pub fn check(&self) -> CheckResult {
        if !can_self_update() {
            return CheckResult::unavailable();
        }
        let Some(arch) = arch_slug() else {
            return CheckResult::unavailable();
        };
        let response = self
            .agent
            .get(RELEASES_API)
            .header("Accept", "application/vnd.github+json")
            .config()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .call();

        let mut response = match response {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(404)) => {
                self.cleanup_old(None);
                return CheckResult::up_to_date();
            }
            Err(ureq::Error::StatusCode(code)) => {
                return CheckResult::failed(format!(
                    "GitHub answered {code} for the release feed."
                ));
            }
            Err(error) => return CheckResult::failed(error.to_string()),
        };

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return CheckResult::failed(format!("GitHub answered {status} for the release feed."));
        }

        let body = match response.body_mut().read_to_string() {
            Ok(body) => body,
            Err(error) => return CheckResult::failed(error.to_string()),
        };
        let result = evaluate_release(&body, &self.current_version, self.install_kind, arch);
        match result.status {
            CheckStatus::Available => {
                if let Some(info) = result.update.as_ref() {
                    self.cleanup_old(Some(&info.version));
                }
            }
            CheckStatus::UpToDate => self.cleanup_old(None),
            _ => {}
        }
        result
    }

    pub fn find_downloaded(&self, info: &UpdateInfo) -> Option<PathBuf> {
        let path = self
            .updates_root
            .join(safe_directory_name(&info.version))
            .join(&info.asset_name);
        let metadata = fs::metadata(&path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        if info.asset_size > 0 && metadata.len() != info.asset_size {
            return None;
        }
        Some(path)
    }

    pub fn download(
        &self,
        info: &UpdateInfo,
        progress: &mut dyn FnMut(DownloadProgress),
        cancel: &AtomicBool,
    ) -> Result<PathBuf, UpdateError> {
        if !can_self_update() {
            return Err(UpdateError::Unavailable(
                "This build does not update itself.".to_string(),
            ));
        }
        if !is_trusted_download_url(&info.asset_url) {
            return Err(UpdateError::UnsafeUrl(info.asset_url.clone()));
        }

        let directory = self.updates_root.join(safe_directory_name(&info.version));
        fs::create_dir_all(&directory)?;
        let file_path = directory.join(&info.asset_name);
        let partial_path = directory.join(format!("{}.part", info.asset_name));

        if file_path.exists() {
            let complete =
                info.asset_size > 0 && fs::metadata(&file_path)?.len() == info.asset_size;
            if complete && self.verify(info, &file_path) {
                return Ok(file_path);
            }
            let _ = fs::remove_file(&file_path);
        }
        let _ = fs::remove_file(&partial_path);

        let mut response = match self.agent.get(&info.asset_url).call() {
            Ok(response) => response,
            Err(error) => {
                let _ = fs::remove_file(&partial_path);
                return Err(error.into());
            }
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let _ = fs::remove_file(&partial_path);
            return Err(UpdateError::Rejected(status));
        }

        let body = response.body_mut();
        let total = body
            .content_length()
            .or_else(|| (info.asset_size > 0).then_some(info.asset_size));
        progress(DownloadProgress { received: 0, total });

        let mut file = fs::File::create(&partial_path)?;
        let mut reader = body.as_reader();
        let mut buffer = [0u8; 64 * 1024];
        let mut received: u64 = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = fs::remove_file(&partial_path);
                return Err(UpdateError::Cancelled);
            }
            let read = match reader.read(&mut buffer) {
                Ok(read) => read,
                Err(error) => {
                    drop(file);
                    let _ = fs::remove_file(&partial_path);
                    return Err(error.into());
                }
            };
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])?;
            received += read as u64;
            progress(DownloadProgress { received, total });
        }
        file.flush()?;
        drop(file);

        if file_path.exists() {
            let _ = fs::remove_file(&file_path);
        }
        fs::rename(&partial_path, &file_path)?;

        if !self.verify(info, &file_path) {
            let _ = fs::remove_file(&file_path);
            return Err(UpdateError::Integrity(
                "The downloaded update failed its integrity check. Try the download again."
                    .to_string(),
            ));
        }
        Ok(file_path)
    }

    pub fn install(&self, info: &UpdateInfo, downloaded: &Path) -> Result<(), UpdateError> {
        if !downloaded.is_file() {
            return Err(UpdateError::Io(
                "The downloaded update is missing.".to_string(),
            ));
        }
        if !self.verify(info, downloaded) {
            return Err(UpdateError::Integrity(
                "The downloaded update failed its integrity check. Download it again.".to_string(),
            ));
        }
        match info.install_kind {
            InstallKind::Installer => start_installer(downloaded),
            InstallKind::Portable => start_portable_update(downloaded),
        }
    }

    pub fn verify(&self, info: &UpdateInfo, file: &Path) -> bool {
        let checksums = match self
            .agent
            .get(&info.checksums_url)
            .config()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .call()
        {
            Ok(response) => {
                let mut body = response.into_body();
                match body.read_to_string() {
                    Ok(text) => text,
                    Err(_) => return false,
                }
            }
            Err(_) => return false,
        };
        let Some(expected) = checksum_for(&checksums, &info.asset_name) else {
            return false;
        };
        match sha256_file(file) {
            Ok(actual) => actual.eq_ignore_ascii_case(&expected),
            Err(_) => false,
        }
    }

    fn cleanup_old(&self, keep_version: Option<&str>) {
        let Ok(entries) = fs::read_dir(&self.updates_root) else {
            return;
        };
        let keep = keep_version.map(safe_directory_name);
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            if keep.as_deref() == Some(name.as_str()) {
                continue;
            }
            let _ = fs::remove_dir_all(&path);
        }
    }
}

pub fn can_self_update() -> bool {
    if !cfg!(windows) {
        return false;
    }
    if cfg!(debug_assertions) {
        return std::env::var("LARIK_ALLOW_SELF_UPDATE").is_ok_and(|value| value.trim() == "1");
    }
    true
}

fn build_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!(
            "Larik-SQL-Studio/{} (+https://github.com/{REPOSITORY})",
            env!("CARGO_PKG_VERSION")
        ))
        .https_only(true)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .build()
        .into()
}

fn arch_slug() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

fn asset_suffix(kind: InstallKind, arch: &str) -> String {
    match kind {
        InstallKind::Installer => format!("-windows-{arch}-setup.exe"),
        InstallKind::Portable => format!("-windows-{arch}-portable.zip"),
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseRecord {
    tag_name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize, Clone)]
struct ReleaseAsset {
    name: Option<String>,
    browser_download_url: Option<String>,
    #[serde(default)]
    size: u64,
}

fn evaluate_release(
    body: &str,
    current_version: &str,
    kind: InstallKind,
    arch: &str,
) -> CheckResult {
    let release: ReleaseRecord = match serde_json::from_str(body) {
        Ok(release) => release,
        Err(error) => return CheckResult::failed(error.to_string()),
    };
    if release.draft || release.prerelease || release.assets.is_empty() {
        return CheckResult::up_to_date();
    }

    let Some(tag) = release.tag_name.as_deref() else {
        return CheckResult::up_to_date();
    };
    let version_text = tag.trim().trim_start_matches(['v', 'V']).to_string();
    let Some(candidate) = parse_version(&version_text) else {
        return CheckResult::up_to_date();
    };
    let Some(current) = parse_version(current_version) else {
        return CheckResult::unavailable();
    };
    if candidate <= current {
        return CheckResult::up_to_date();
    }

    let suffix = asset_suffix(kind, arch);
    let expected = format!("Larik-SQL-Studio-v{version_text}{suffix}");
    let asset = release
        .assets
        .iter()
        .find(|asset| {
            asset
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&expected))
        })
        .or_else(|| {
            release.assets.iter().find(|asset| {
                asset.name.as_deref().is_some_and(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower.ends_with(&suffix.to_ascii_lowercase())
                        && lower.contains(&version_text.to_ascii_lowercase())
                })
            })
        });

    let Some(asset) = asset else {
        return CheckResult::failed(format!(
            "Larik {version_text} has no downloadable {arch} build."
        ));
    };
    let Some(asset_name) = asset.name.as_deref() else {
        return CheckResult::failed("The release asset has no name.");
    };
    if !is_plain_file_name(asset_name) {
        return CheckResult::failed("The release asset has an unsafe name.");
    }
    let Some(asset_url) = asset.browser_download_url.as_deref() else {
        return CheckResult::failed("The release asset has no download URL.");
    };
    if !is_trusted_download_url(asset_url) {
        return CheckResult::failed("The release asset is hosted somewhere untrusted.");
    }

    let checksums_url = release
        .assets
        .iter()
        .find(|asset| {
            asset
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(CHECKSUMS_ASSET))
        })
        .and_then(|asset| asset.browser_download_url.clone())
        .filter(|url| is_trusted_download_url(url));

    let Some(checksums_url) = checksums_url else {
        return CheckResult::failed(format!(
            "Larik {version_text} has no {CHECKSUMS_ASSET}, so it cannot be verified. Download it manually from GitHub Releases."
        ));
    };

    CheckResult::available(UpdateInfo {
        version: version_text,
        asset_name: asset_name.to_string(),
        asset_url: asset_url.to_string(),
        asset_size: asset.size,
        checksums_url,
        install_kind: kind,
    })
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let trimmed = text.trim().trim_start_matches(['v', 'V']);
    let numeric = trimmed.split(['-', '+']).next()?.trim();
    if numeric.is_empty() {
        return None;
    }
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts
        .next()
        .map(|part| part.parse().ok())
        .unwrap_or(Some(0))?;
    let patch = parts
        .next()
        .map(|part| part.parse().ok())
        .unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name).file_name().and_then(|part| part.to_str()) == Some(name)
        && !name.contains("..")
}

fn safe_directory_name(value: &str) -> String {
    let mut cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '.' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", "__");
    }
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '_') {
        "update".to_string()
    } else {
        cleaned
    }
}

fn is_trusted_download_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    if rest.contains('@') {
        return false;
    }
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    host == "github.com"
        || host.ends_with(".github.com")
        || host == "githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
}

fn checksum_for(checksums: &str, asset_name: &str) -> Option<String> {
    checksums
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (parts.next().is_none() && name.eq_ignore_ascii_case(asset_name))
                .then(|| hash.to_string())
        })
        .find(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

fn app_directory() -> Result<PathBuf, UpdateError> {
    let exe = std::env::current_exe()
        .map_err(|error| UpdateError::Io(format!("Could not find this copy of Larik: {error}")))?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| UpdateError::Io("Could not find this copy of Larik.".to_string()))
}

fn start_installer(installer_path: &Path) -> Result<(), UpdateError> {
    let target = app_directory()?;
    let directory = installer_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| target.clone());
    let log_path = directory.join("install.log");

    let mut command = Command::new(installer_path);
    command
        .arg("/SILENT")
        .arg("/NORESTART")
        .arg("/CLOSEAPPLICATIONS")
        .arg("/NORESTARTAPPLICATIONS")
        .arg("/RELAUNCH=1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.raw_arg(format!("/DIR=\"{}\"", target.display()));
        command.raw_arg(format!("/LOG=\"{}\"", log_path.display()));
    }
    command.current_dir(&directory);
    command.spawn().map(|_| ()).map_err(|error| {
        UpdateError::Io(format!(
            "Windows didn't start the update installer: {error}"
        ))
    })
}

fn start_portable_update(zip_path: &Path) -> Result<(), UpdateError> {
    let update_directory = zip_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| UpdateError::Io("The downloaded update is incomplete.".to_string()))?;
    let target = app_directory()?;

    ensure_writable(&target)?;

    let staging = update_directory.join("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    {
        let file = fs::File::open(zip_path)
            .map_err(|error| UpdateError::Io(format!("Could not read the update: {error}")))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|error| {
            UpdateError::Integrity(format!("The update ZIP is corrupt: {error}"))
        })?;
        archive
            .extract(&staging)
            .map_err(|error| UpdateError::Io(format!("Could not unpack the update: {error}")))?;
    }
    if !staging.join(APP_EXE).is_file() {
        return Err(UpdateError::Integrity(format!(
            "The downloaded ZIP doesn't contain {APP_EXE}."
        )));
    }

    let script_path = update_directory.join("apply-update.cmd");
    let script = PORTABLE_APPLY_SCRIPT.replace('\n', "\r\n");
    fs::write(&script_path, script.as_bytes())?;

    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let cmd_exe = Path::new(&system_root).join("System32").join("cmd.exe");
    let mut command = Command::new(cmd_exe);
    command
        .arg("/d")
        .arg("/c")
        .arg(format!("\"{}\"", script_path.display()))
        .current_dir(&update_directory);
    command
        .env("LARIK_UPDATE_PID", std::process::id().to_string())
        .env("LARIK_UPDATE_SOURCE", &staging)
        .env("LARIK_UPDATE_TARGET", &target)
        .env("LARIK_UPDATE_EXE", target.join(APP_EXE));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map(|_| ()).map_err(|error| {
        UpdateError::Io(format!(
            "Windows didn't start the portable updater: {error}"
        ))
    })
}

fn ensure_writable(directory: &Path) -> Result<(), UpdateError> {
    let probe = directory.join(format!(".larik-{}.tmp", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(error) => Err(UpdateError::Io(format!(
            "Larik can't write to the folder it runs from, so this portable copy can't update in place. Download the new ZIP from GitHub Releases instead. ({error})"
        ))),
    }
}

fn detect_install_kind() -> InstallKind {
    let Ok(directory) = app_directory() else {
        return InstallKind::Portable;
    };
    if directory.join("unins000.exe").is_file() {
        return InstallKind::Installer;
    }
    if installed_location_matches(&directory) {
        return InstallKind::Installer;
    }
    InstallKind::Portable
}

#[cfg(windows)]
fn installed_location_matches(directory: &Path) -> bool {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{A7C4F8E2-3D51-4B9E-8F1A-6C2E9D4A1B73}_is1";

    let Ok(target) = fs::canonicalize(directory) else {
        return false;
    };
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let root = RegKey::predef(hive);
        let Ok(key) = root.open_subkey_with_flags(KEY, KEY_READ) else {
            continue;
        };
        let Ok(location) = key.get_value::<String, _>("InstallLocation") else {
            continue;
        };
        if location.trim().is_empty() {
            continue;
        }
        if let Ok(location) = fs::canonicalize(location.trim())
            && location == target
        {
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
fn installed_location_matches(_directory: &Path) -> bool {
    false
}

const PORTABLE_APPLY_SCRIPT: &str = r#"@echo off
setlocal
:wait_for_larik
"%SystemRoot%\System32\tasklist.exe" /FI "PID eq %LARIK_UPDATE_PID%" /NH 2>nul | "%SystemRoot%\System32\find.exe" "%LARIK_UPDATE_PID%" >nul
if not errorlevel 1 (
    "%SystemRoot%\System32\ping.exe" -n 2 127.0.0.1 >nul
    goto wait_for_larik
)
"%SystemRoot%\System32\robocopy.exe" "%LARIK_UPDATE_SOURCE%" "%LARIK_UPDATE_TARGET%" /E /IS /IT /R:3 /W:1 /NFL /NDL /NJH /NJS /NP > "%~dp0robocopy.log" 2>&1
start "" "%LARIK_UPDATE_EXE%"
endlocal
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json(version: &str) -> String {
        format!(
            r#"{{"tag_name":"v{version}","draft":false,"prerelease":false,"assets":[{{"name":"Larik-SQL-Studio-v{version}-windows-x64-setup.exe","browser_download_url":"https://github.com/dwifuady/larik-sql-studio/releases/download/v{version}/Larik-SQL-Studio-v{version}-windows-x64-setup.exe","size":123}},{{"name":"Larik-SQL-Studio-v{version}-windows-x64-portable.zip","browser_download_url":"https://github.com/dwifuady/larik-sql-studio/releases/download/v{version}/Larik-SQL-Studio-v{version}-windows-x64-portable.zip","size":456}},{{"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/dwifuady/larik-sql-studio/releases/download/v{version}/SHA256SUMS.txt","size":99}}]}}"#
        )
    }

    #[test]
    fn version_parsing_strips_prefix_and_suffix() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.11"), Some((0, 11, 0)));
        assert_eq!(parse_version("2.0.0-beta.1"), Some((2, 0, 0)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("not-a-version"), None);
    }

    #[test]
    fn newer_versions_are_offered_with_the_matching_installer() {
        let result = evaluate_release(
            &release_json("0.12.0"),
            "0.11.0",
            InstallKind::Installer,
            "x64",
        );
        assert_eq!(result.status, CheckStatus::Available);
        let info = result.update.expect("available update");
        assert_eq!(info.version, "0.12.0");
        assert_eq!(
            info.asset_name,
            "Larik-SQL-Studio-v0.12.0-windows-x64-setup.exe"
        );
        assert_eq!(info.asset_size, 123);
        assert_eq!(info.install_kind, InstallKind::Installer);
        assert!(info.checksums_url.ends_with("SHA256SUMS.txt"));
    }

    #[test]
    fn newer_versions_are_offered_with_the_matching_portable_zip() {
        let result = evaluate_release(
            &release_json("0.12.0"),
            "0.11.0",
            InstallKind::Portable,
            "x64",
        );
        assert_eq!(result.status, CheckStatus::Available);
        let info = result.update.expect("available update");
        assert_eq!(
            info.asset_name,
            "Larik-SQL-Studio-v0.12.0-windows-x64-portable.zip"
        );
        assert_eq!(info.asset_size, 456);
    }

    #[test]
    fn same_or_older_versions_are_up_to_date() {
        for tag in ["0.11.0", "0.10.9", "0.11"] {
            let result =
                evaluate_release(&release_json(tag), "0.11.0", InstallKind::Portable, "x64");
            assert_eq!(result.status, CheckStatus::UpToDate, "tag {tag}");
        }
    }

    #[test]
    fn drafts_and_prereleases_are_ignored() {
        let draft = release_json("0.12.0").replace("\"draft\":false", "\"draft\":true");
        assert_eq!(
            evaluate_release(&draft, "0.11.0", InstallKind::Portable, "x64").status,
            CheckStatus::UpToDate
        );
        let pre = release_json("0.12.0").replace("\"prerelease\":false", "\"prerelease\":true");
        assert_eq!(
            evaluate_release(&pre, "0.11.0", InstallKind::Portable, "x64").status,
            CheckStatus::UpToDate
        );
    }

    #[test]
    fn missing_checksums_block_the_update() {
        let json = r#"{"tag_name":"v0.12.0","assets":[{"name":"Larik-SQL-Studio-v0.12.0-windows-x64-portable.zip","browser_download_url":"https://github.com/dwifuady/larik-sql-studio/releases/download/v0.12.0/Larik-SQL-Studio-v0.12.0-windows-x64-portable.zip","size":1}]}"#;
        let result = evaluate_release(json, "0.11.0", InstallKind::Portable, "x64");
        assert_eq!(result.status, CheckStatus::Failed);
        assert!(result.error.unwrap_or_default().contains("SHA256SUMS.txt"));
    }

    #[test]
    fn assets_from_untrusted_hosts_are_refused() {
        let json = r#"{"tag_name":"v0.12.0","assets":[{"name":"Larik-SQL-Studio-v0.12.0-windows-x64-portable.zip","browser_download_url":"https://example.com/evil.zip","size":1},{"name":"SHA256SUMS.txt","browser_download_url":"https://example.com/SHA256SUMS.txt","size":1}]}"#;
        let result = evaluate_release(json, "0.11.0", InstallKind::Portable, "x64");
        assert_eq!(result.status, CheckStatus::Failed);
    }

    #[test]
    fn trusted_download_hosts() {
        assert!(is_trusted_download_url(
            "https://github.com/dwifuady/larik-sql-studio/releases/download/v1/x.zip"
        ));
        assert!(is_trusted_download_url(
            "https://objects.githubusercontent.com/a/b"
        ));
        assert!(!is_trusted_download_url("http://github.com/a/b"));
        assert!(!is_trusted_download_url("https://evil.example.com/a/b"));
        assert!(!is_trusted_download_url(
            "https://github.com@evil.example.com/a/b"
        ));
    }

    #[test]
    fn checksum_lookup_matches_the_asset() {
        let digest = "b".repeat(64);
        let sums = format!(
            "\u{feff}{}  other.zip\r\n{digest}  *Larik-SQL-Studio-v0.12.0-windows-x64-portable.zip\r\ncccc  last.zip\r\n",
            "a".repeat(64)
        );
        assert_eq!(checksum_for(&sums, "other.zip"), Some("a".repeat(64)));
        assert_eq!(
            checksum_for(&sums, "larik-sql-studio-v0.12.0-windows-x64-portable.zip"),
            Some(digest)
        );
        assert_eq!(checksum_for(&sums, "missing.zip"), None);
    }

    #[test]
    fn checksum_entries_require_two_columns_and_a_hex_digest() {
        let sums = format!(
            "{}  Larik.zip\r\n{}  other.zip\r\n",
            "f".repeat(64),
            "g".repeat(64)
        );
        assert_eq!(checksum_for(&sums, "larik.zip"), Some("f".repeat(64)));
        assert_eq!(checksum_for(&sums, "other.zip"), None);
    }

    #[test]
    fn download_percent_is_clamped_and_optional() {
        let progress = DownloadProgress {
            received: 50,
            total: Some(200),
        };
        assert_eq!(progress.percent(), Some(25.0));
        let unknown = DownloadProgress {
            received: 50,
            total: None,
        };
        assert_eq!(unknown.percent(), None);
    }

    #[test]
    fn directory_names_are_sanitized() {
        assert_eq!(safe_directory_name("0.12.0"), "0.12.0");
        assert_eq!(safe_directory_name("../1.0"), "___1.0");
        assert_eq!(safe_directory_name("a/b\\c"), "a_b_c");
        assert_eq!(safe_directory_name(".."), "update");
        assert_eq!(safe_directory_name(""), "update");
    }

    #[test]
    fn plain_file_names_are_required() {
        assert!(is_plain_file_name(
            "Larik-SQL-Studio-v0.12.0-windows-x64-portable.zip"
        ));
        assert!(!is_plain_file_name("../evil.zip"));
        assert!(!is_plain_file_name("dir/evil.zip"));
        assert!(!is_plain_file_name(""));
    }
}
