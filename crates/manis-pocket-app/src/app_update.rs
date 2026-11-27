//! Self-update check, verified download, and macOS in-place bundle swap.
//!
//! Mirrors the update architecture used by Manis:
//! - Checks GitHub Releases over HTTPS-only `ureq` with bounded metadata reads.
//! - Verifies the SHA-256 digest published by GitHub before touching the disk.
//! - On macOS arm64 (matching the CI release artifact), extracts the `.zip` in
//!   pure Rust and swaps the `.app` bundle via a detached shell script before
//!   relaunching with `open -n`. Because the archive is fetched and unpacked by
//!   the app itself rather than a browser, macOS does not attach a quarantine
//!   attribute to the replacement bundle.

use std::error::Error;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use ureq::{Agent, ResponseExt as _};

const RELEASES_API: &str =
    "https://api.github.com/repos/kaigedong/Manis-Pocket/releases?per_page=20";
pub(crate) const REPOSITORY_URL: &str = "https://github.com/kaigedong/Manis-Pocket";
const MACOS_ASSET_NAME: &str = "ManisPocket.app.zip";
const MAX_RELEASE_METADATA_BYTES: u64 = 1024 * 1024;
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
const PACKAGE_TIMEOUT: Duration = Duration::from_secs(600);
const DOWNLOAD_CHUNK_BYTES: usize = 32 * 1024;
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(100);
const MAX_REDIRECTS: u32 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Version([u64; 3]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReleaseAsset {
    pub(crate) name: String,
    pub(crate) download_url: String,
    pub(crate) sha256: String,
    pub(crate) size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AvailableUpdate {
    pub(crate) version: String,
    pub(crate) tag_name: String,
    pub(crate) release_url: String,
    pub(crate) asset: Option<ReleaseAsset>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DownloadProgress {
    pub(crate) downloaded_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) bytes_per_second: u64,
}

impl DownloadProgress {
    pub(crate) fn percent(self) -> Option<u8> {
        let total = self.total_bytes.filter(|total| *total > 0)?;
        let scaled = u128::from(self.downloaded_bytes.min(total)) * 100 / u128::from(total);
        u8::try_from(scaled).ok()
    }

    pub(crate) fn label(self) -> String {
        let prefix = "正在下载...";
        let speed =
            (self.bytes_per_second > 0).then(|| format_transfer_speed(self.bytes_per_second));
        if let Some(percent) = self.percent() {
            return match speed {
                Some(speed) => format!("{prefix} {percent}% · {speed}"),
                None => format!("{prefix} {percent}%"),
            };
        }
        if self.downloaded_bytes > 0 {
            let downloaded = format_compact_size(self.downloaded_bytes);
            return match speed {
                Some(speed) => format!("{prefix} {downloaded} · {speed}"),
                None => format!("{prefix} {downloaded}"),
            };
        }
        prefix.to_owned()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppUpdateError {
    NetworkUnavailable,
    InvalidMetadata,
    InsecureRedirect,
    InvalidDigest,
    DigestMismatch,
    MetadataTooLarge,
    MissingPackage,
    UnsupportedPlatform,
    PackageTooLarge,
}

impl AppUpdateError {
    pub(crate) const fn user_message(self) -> &'static str {
        match self {
            Self::NetworkUnavailable => "暂时无法检查更新，可前往 GitHub 查看。",
            Self::InvalidMetadata
            | Self::InsecureRedirect
            | Self::InvalidDigest
            | Self::DigestMismatch
            | Self::MetadataTooLarge => "无法验证版本信息，可前往 GitHub 查看。",
            Self::MissingPackage | Self::UnsupportedPlatform => {
                "当前构建没有可安装的包，可前往 GitHub 查看。"
            }
            Self::PackageTooLarge => "更新包超出大小限制，可前往 GitHub 查看。",
        }
    }
}

impl fmt::Display for AppUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NetworkUnavailable => "the update service is unavailable",
            Self::InvalidMetadata => "the update metadata is invalid",
            Self::InsecureRedirect => "the update request left HTTPS",
            Self::InvalidDigest => "the update checksum is invalid",
            Self::DigestMismatch => "the update package failed verification",
            Self::MetadataTooLarge => "the update metadata exceeds the safety limit",
            Self::MissingPackage => "the release has no package for this platform",
            Self::UnsupportedPlatform => "this platform cannot install updates itself",
            Self::PackageTooLarge => "the update package exceeds the safety limit",
        })
    }
}

impl Error for AppUpdateError {}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

/// The version string baked into this binary by `build.rs`.
pub(crate) const fn current_version() -> &'static str {
    env!("MANIS_POCKET_BUILD_VERSION")
}

/// Whether this build can download and install an update in place.
///
/// CI builds `ManisPocket.app.zip` on Apple Silicon macOS (`aarch64`). Intel
/// Macs and Linux still check for updates and link straight to the GitHub
/// release page instead of attempting to install an incompatible binary.
pub(crate) const fn supports_self_install() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// Checks GitHub Releases for a version newer than `current_version()`.
pub(crate) fn check_for_update() -> Result<Option<AvailableUpdate>, AppUpdateError> {
    let release_bytes = download_bytes(RELEASES_API, MAX_RELEASE_METADATA_BYTES)?;
    select_available_update(&release_bytes, current_version(), supports_self_install())
}

fn select_available_update(
    releases_bytes: &[u8],
    installed_version: &str,
    require_package: bool,
) -> Result<Option<AvailableUpdate>, AppUpdateError> {
    let releases: Vec<GithubRelease> =
        serde_json::from_slice(releases_bytes).map_err(|_error| AppUpdateError::InvalidMetadata)?;
    let installed = Version::parse_installed(installed_version)?;

    let mut best: Option<(Version, AvailableUpdate)> = None;
    let mut saw_newer_without_package = false;

    for release in releases {
        if release.draft {
            continue;
        }
        let Some((version, version_text)) = Version::parse_release_tag(&release.tag_name) else {
            continue;
        };
        if version <= installed {
            continue;
        }

        let asset = extract_macos_asset(&release.assets);
        if require_package && asset.is_none() {
            saw_newer_without_package = true;
            continue;
        }

        let release_url = release
            .html_url
            .filter(|url| url.starts_with("https://"))
            .unwrap_or_else(|| format!("{REPOSITORY_URL}/releases/tag/{}", release.tag_name));

        let candidate = AvailableUpdate {
            version: version_text,
            tag_name: release.tag_name,
            release_url,
            asset,
        };

        if best
            .as_ref()
            .is_none_or(|(best_version, _)| version > *best_version)
        {
            best = Some((version, candidate));
        }
    }

    if let Some((_, update)) = best {
        return Ok(Some(update));
    }
    if saw_newer_without_package {
        return Err(AppUpdateError::MissingPackage);
    }
    Ok(None)
}

fn extract_macos_asset(assets: &[GithubAsset]) -> Option<ReleaseAsset> {
    let asset = assets.iter().find(|asset| asset.name == MACOS_ASSET_NAME)?;
    if !asset.browser_download_url.starts_with("https://") {
        return None;
    }
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| parse_github_digest(digest).ok())?;
    Some(ReleaseAsset {
        name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        sha256,
        size: asset.size,
    })
}

impl Version {
    fn parse_exact(value: &str) -> Result<Self, AppUpdateError> {
        let mut components = value.split('.');
        let major = parse_version_component(components.next())?;
        let minor = parse_version_component(components.next())?;
        let patch = parse_version_component(components.next())?;
        if components.next().is_some() {
            return Err(AppUpdateError::InvalidMetadata);
        }
        Ok(Self([major, minor, patch]))
    }

    /// Parses either a release version (`2.7.142`) or a local VCS build version
    /// produced by `build.rs` (`0.1.0.r1136.gf85a27d`).
    fn parse_installed(value: &str) -> Result<Self, AppUpdateError> {
        let trimmed = value.trim();
        let trimmed = trimmed.strip_prefix('v').unwrap_or(trimmed);
        if let Ok(exact) = Self::parse_exact(trimmed) {
            return Ok(exact);
        }
        let mut components = trimmed.split('.');
        let major = parse_version_component(components.next())?;
        let minor = parse_version_component(components.next())?;
        let patch = parse_version_component(components.next())?;
        let rev = components.next();
        let git = components.next();
        if components.next().is_some() {
            return Err(AppUpdateError::InvalidMetadata);
        }
        let valid_vcs = rev
            .and_then(|part| part.strip_prefix('r'))
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            && git
                .and_then(|part| part.strip_prefix('g'))
                .is_some_and(|hex| {
                    !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                });
        if valid_vcs {
            Ok(Self([major, minor, patch]))
        } else {
            Err(AppUpdateError::InvalidMetadata)
        }
    }

    fn parse_release_tag(tag: &str) -> Option<(Self, String)> {
        let stripped = tag
            .strip_prefix("build-")
            .or_else(|| tag.strip_prefix('v'))?;
        let version = Self::parse_exact(stripped).ok()?;
        Some((version, stripped.to_owned()))
    }
}

fn parse_version_component(component: Option<&str>) -> Result<u64, AppUpdateError> {
    component
        .filter(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
        .and_then(|component| component.parse().ok())
        .ok_or(AppUpdateError::InvalidMetadata)
}

fn download_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>, AppUpdateError> {
    let agent = https_agent();
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| map_request_error(&error))?;
    require_https(&response)?;
    read_limited_body(&mut response, max_bytes)
}

fn read_limited_body(
    response: &mut ureq::http::Response<ureq::Body>,
    max_bytes: u64,
) -> Result<Vec<u8>, AppUpdateError> {
    response
        .body_mut()
        .with_config()
        .limit(max_bytes + 1)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => AppUpdateError::MetadataTooLarge,
            _ => AppUpdateError::NetworkUnavailable,
        })
        .and_then(|bytes| enforce_body_limit(bytes, max_bytes))
}

fn enforce_body_limit(bytes: Vec<u8>, max_bytes: u64) -> Result<Vec<u8>, AppUpdateError> {
    if bytes.len() as u64 > max_bytes {
        Err(AppUpdateError::MetadataTooLarge)
    } else {
        Ok(bytes)
    }
}

fn https_agent() -> Agent {
    Agent::config_builder()
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .timeout_global(Some(METADATA_TIMEOUT))
        .user_agent(concat!(
            "Manis-Pocket/",
            env!("CARGO_PKG_VERSION"),
            " App-Update-Check"
        ))
        .build()
        .into()
}

fn require_https(response: &ureq::http::Response<ureq::Body>) -> Result<(), AppUpdateError> {
    if response.get_uri().scheme_str() == Some("https") {
        Ok(())
    } else {
        Err(AppUpdateError::InsecureRedirect)
    }
}

fn map_request_error(error: &ureq::Error) -> AppUpdateError {
    match error {
        ureq::Error::RequireHttpsOnly(_) => AppUpdateError::InsecureRedirect,
        _ => AppUpdateError::NetworkUnavailable,
    }
}

fn parse_github_digest(value: &str) -> Result<String, AppUpdateError> {
    let digest = value
        .strip_prefix("sha256:")
        .ok_or(AppUpdateError::InvalidDigest)?;
    if is_sha256(digest) {
        Ok(digest.to_ascii_lowercase())
    } else {
        Err(AppUpdateError::InvalidDigest)
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn verify_digest(bytes: &[u8], expected: &str) -> Result<(), AppUpdateError> {
    if !is_sha256(expected) {
        return Err(AppUpdateError::InvalidDigest);
    }
    let actual = hex::encode(Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(AppUpdateError::DigestMismatch)
    }
}

/// Downloads the package for this release, reporting streaming progress and verifying its SHA-256 digest.
pub(crate) fn download_package_with_progress(
    update: &AvailableUpdate,
    on_progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf, AppUpdateError> {
    if !supports_self_install() {
        return Err(AppUpdateError::UnsupportedPlatform);
    }
    let asset = update
        .asset
        .as_ref()
        .ok_or(AppUpdateError::MissingPackage)?;
    if !asset.download_url.starts_with("https://") {
        return Err(AppUpdateError::InsecureRedirect);
    }
    let bytes = download_package_bytes(&asset.download_url, asset.size, on_progress)?;
    verify_digest(&bytes, &asset.sha256)?;

    let directory =
        std::env::temp_dir().join(format!("manis-pocket-app-update-{}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|_error| AppUpdateError::NetworkUnavailable)?;
    let package = directory.join(&asset.name);
    std::fs::write(&package, bytes).map_err(|_error| AppUpdateError::NetworkUnavailable)?;
    Ok(package)
}

fn download_package_bytes(
    url: &str,
    fallback_total_bytes: Option<u64>,
    on_progress: impl FnMut(DownloadProgress),
) -> Result<Vec<u8>, AppUpdateError> {
    let agent: Agent = Agent::config_builder()
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .timeout_global(Some(PACKAGE_TIMEOUT))
        .user_agent(concat!(
            "Manis-Pocket/",
            env!("CARGO_PKG_VERSION"),
            " App-Update"
        ))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| map_request_error(&error))?;
    require_https(&response)?;
    let header_total_bytes = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|total| *total > 0);
    let total_bytes =
        header_total_bytes.or_else(|| fallback_total_bytes.filter(|total| *total > 0));
    let mut reader = response.body_mut().as_reader();
    read_package_stream(&mut reader, total_bytes, MAX_PACKAGE_BYTES, on_progress)
}

fn read_package_stream(
    reader: &mut impl Read,
    total_bytes: Option<u64>,
    max_bytes: u64,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<Vec<u8>, AppUpdateError> {
    if total_bytes.is_some_and(|total| total > max_bytes) {
        return Err(AppUpdateError::PackageTooLarge);
    }
    let initial_capacity = total_bytes
        .and_then(|total| usize::try_from(total.min(max_bytes)).ok())
        .unwrap_or(0);
    let mut bytes = Vec::with_capacity(initial_capacity);
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_BYTES];
    let started_at = Instant::now();
    let mut last_emit = started_at;
    let mut last_sample_at = started_at;
    let mut last_sample_bytes = 0_u64;
    let mut smoothed_speed = 0_u64;

    on_progress(DownloadProgress {
        downloaded_bytes: 0,
        total_bytes,
        bytes_per_second: 0,
    });

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_error| AppUpdateError::NetworkUnavailable)?;
        if read == 0 {
            break;
        }
        let next_len = bytes
            .len()
            .checked_add(read)
            .ok_or(AppUpdateError::PackageTooLarge)?;
        if next_len as u64 > max_bytes {
            return Err(AppUpdateError::PackageTooLarge);
        }
        bytes.extend_from_slice(&buffer[..read]);
        let downloaded = bytes.len() as u64;
        let now = Instant::now();
        if now.duration_since(last_emit) >= PROGRESS_EMIT_INTERVAL {
            smoothed_speed = compute_transfer_speed(
                downloaded,
                last_sample_bytes,
                now.duration_since(last_sample_at),
                now.duration_since(started_at),
                smoothed_speed,
            );
            last_sample_bytes = downloaded;
            last_sample_at = now;
            last_emit = now;
            on_progress(DownloadProgress {
                downloaded_bytes: downloaded,
                total_bytes,
                bytes_per_second: smoothed_speed,
            });
        }
    }

    let downloaded = bytes.len() as u64;
    let now = Instant::now();
    let final_speed = compute_transfer_speed(
        downloaded,
        last_sample_bytes,
        now.duration_since(last_sample_at),
        now.duration_since(started_at),
        smoothed_speed,
    );
    on_progress(DownloadProgress {
        downloaded_bytes: downloaded,
        total_bytes: total_bytes.or(Some(downloaded)),
        bytes_per_second: final_speed,
    });
    Ok(bytes)
}

fn compute_transfer_speed(
    downloaded_bytes: u64,
    last_sample_bytes: u64,
    window_elapsed: Duration,
    total_elapsed: Duration,
    previous_speed: u64,
) -> u64 {
    let delta = downloaded_bytes.saturating_sub(last_sample_bytes);
    let instant_speed = rate_bytes_per_second(delta, window_elapsed)
        .or_else(|| rate_bytes_per_second(downloaded_bytes, total_elapsed))
        .unwrap_or(previous_speed);
    if previous_speed == 0 {
        instant_speed
    } else {
        (previous_speed * 2 + instant_speed) / 3
    }
}

fn rate_bytes_per_second(bytes: u64, elapsed: Duration) -> Option<u64> {
    let millis = elapsed.as_millis();
    if bytes == 0 || millis == 0 {
        return None;
    }
    let scaled = u128::from(bytes) * 1_000 / millis;
    u64::try_from(scaled).ok()
}

fn format_transfer_speed(bytes_per_second: u64) -> String {
    format!("{}/s", format_compact_size(bytes_per_second))
}

fn format_compact_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    let rounded_tenths =
        |unit: u64| -> u128 { (u128::from(bytes) * 10 + u128::from(unit) / 2) / u128::from(unit) };
    if bytes >= GIB {
        let tenths = rounded_tenths(GIB);
        format!("{}.{} GB", tenths / 10, tenths % 10)
    } else if bytes >= MIB {
        let tenths = rounded_tenths(MIB);
        format!("{}.{} MB", tenths / 10, tenths % 10)
    } else if bytes >= KIB {
        let tenths = rounded_tenths(KIB);
        if tenths >= 1000 {
            format!("{} KB", tenths / 10)
        } else {
            format!("{}.{} KB", tenths / 10, tenths % 10)
        }
    } else {
        format!("{bytes} B")
    }
}

/// Installs a downloaded package and schedules a restart. Returns once the swap script is spawned.
pub(crate) fn install_package(package: &Path) -> Result<(), String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        macos_install::install(package)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = package;
        Err("当前平台不支持自动安装更新".to_owned())
    }
}

// Compiled on every Unix target so package extraction and swap script generation
// stay type-checked and covered by unit tests on Linux CI too.
#[cfg(unix)]
#[allow(dead_code)]
mod macos_install {
    use std::io::{Cursor, Read, Write};
    use std::path::{Path, PathBuf};

    const APP_BUNDLE_NAME: &str = "Manis Pocket.app";
    const APP_EXECUTABLE_REL: &str = "Contents/MacOS/manis-pocket";
    const MAX_ENTRIES: usize = 20_000;
    const MAX_EXTRACTED_BYTES: u64 = 1_024 * 1024 * 1024;

    pub(super) fn install(package: &Path) -> Result<(), String> {
        use std::process::{Command, Stdio};

        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let bundle = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(|| "无法定位当前应用程序包".to_owned())?;
        if bundle
            .extension()
            .is_none_or(|extension| extension != "app")
        {
            return Err("当前不是从 .app 应用程序包启动，无法原地替换更新".to_owned());
        }

        let work_dir = package
            .parent()
            .ok_or_else(|| "更新包路径无效".to_owned())?;
        let staging = work_dir.join("extract");
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
        }
        std::fs::create_dir_all(&staging).map_err(|error| error.to_string())?;

        let bytes = std::fs::read(package).map_err(|error| error.to_string())?;
        extract_zip(&bytes, &staging)?;

        let replacement = find_bundle(&staging)?;
        if !replacement.join(APP_EXECUTABLE_REL).is_file() {
            return Err("更新包中未找到有效的 Manis Pocket 可执行文件".to_owned());
        }

        let script = staging.join("install.sh");
        let script_body = swap_script(std::process::id(), bundle, &replacement, work_dir);
        std::fs::write(&script, script_body).map_err(|error| error.to_string())?;
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .map_err(|error| error.to_string())?;
        }

        Command::new("/bin/sh")
            .arg(&script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn shell_quote(path: &Path) -> String {
        let raw = path.to_string_lossy();
        let mut quoted = String::with_capacity(raw.len() + 2);
        quoted.push('\'');
        for ch in raw.chars() {
            if ch == '\'' {
                quoted.push_str("'\\''");
            } else {
                quoted.push(ch);
            }
        }
        quoted.push('\'');
        quoted
    }

    /// Detached swap script: waits for this process to exit, moves the old bundle
    /// aside, moves the freshly extracted bundle into place (rolling back on
    /// failure), reopens it with `open -n`, and cleans up temporary files.
    pub(super) fn swap_script(
        pid: u32,
        bundle: &Path,
        replacement: &Path,
        work_dir: &Path,
    ) -> String {
        let bundle_q = shell_quote(bundle);
        let replacement_q = shell_quote(replacement);
        let work_dir_q = shell_quote(work_dir);
        format!(
            "#!/bin/sh\n\
             while kill -0 {pid} 2>/dev/null; do sleep 0.2; done\n\
             bundle={bundle_q}\n\
             replacement={replacement_q}\n\
             work_dir={work_dir_q}\n\
             backup=\"${{bundle}}.old.$$\"\n\
             rm -rf \"$backup\"\n\
             mv \"$bundle\" \"$backup\" || exit 1\n\
             mv \"$replacement\" \"$bundle\" || {{ mv \"$backup\" \"$bundle\"; exit 1; }}\n\
             open -n \"$bundle\"\n\
             sleep 2\n\
             rm -rf \"$backup\"\n\
             rm -rf \"$work_dir\"\n"
        )
    }

    pub(super) fn extract_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
        if archive.len() > MAX_ENTRIES {
            return Err("更新包内文件数量超出安全限制".to_owned());
        }
        let mut extracted = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
            let name = entry
                .enclosed_name()
                .ok_or_else(|| "更新包包含不安全的路径".to_owned())?;
            if name.as_os_str().is_empty() {
                continue;
            }
            let target = destination.join(&name);
            if entry.is_dir() {
                std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            if entry.is_symlink() {
                let mut link_target = String::new();
                entry
                    .read_to_string(&mut link_target)
                    .map_err(|error| error.to_string())?;
                if link_target.starts_with('/') {
                    return Err("更新包包含不安全的符号链接".to_owned());
                }
                std::os::unix::fs::symlink(&link_target, &target)
                    .map_err(|error| error.to_string())?;
                continue;
            }
            let mut file = std::fs::File::create(&target).map_err(|error| error.to_string())?;
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = entry.read(&mut buffer).map_err(|error| error.to_string())?;
                if read == 0 {
                    break;
                }
                extracted += read as u64;
                if extracted > MAX_EXTRACTED_BYTES {
                    return Err("更新包解压后大小超出安全限制".to_owned());
                }
                file.write_all(&buffer[..read])
                    .map_err(|error| error.to_string())?;
            }
            {
                use std::os::unix::fs::PermissionsExt as _;
                if let Some(mode) = entry.unix_mode() {
                    let _ =
                        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
                }
            }
        }
        Ok(())
    }

    pub(super) fn find_bundle(staging: &Path) -> Result<PathBuf, String> {
        let direct = staging.join(APP_BUNDLE_NAME);
        if direct.is_dir() {
            return Ok(direct);
        }
        for entry in std::fs::read_dir(staging).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().is_some_and(|name| name == "__MACOSX") {
                continue;
            }
            if path.extension().is_some_and(|extension| extension == "app") {
                return Ok(path);
            }
            let nested = path.join(APP_BUNDLE_NAME);
            if nested.is_dir() {
                return Ok(nested);
            }
        }
        Err("更新包中未找到 .app 应用程序包".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_DIGEST: &str =
        "sha256:f36fc2d77f5416f67d663c1b2bcf44d00823eb30b08c81e0f08835d2252bd1ca";

    fn sample_releases_json() -> Vec<u8> {
        serde_json::json!([
            {
                "tag_name": "build-2.7.145",
                "draft": true,
                "html_url": "https://github.com/kaigedong/Manis-Pocket/releases/tag/build-2.7.145",
                "assets": [{
                    "name": MACOS_ASSET_NAME,
                    "browser_download_url": "https://github.com/kaigedong/Manis-Pocket/releases/download/build-2.7.145/ManisPocket.app.zip",
                    "digest": VALID_DIGEST,
                    "size": 12_000_000
                }]
            },
            {
                "tag_name": "build-2.7.142",
                "draft": false,
                "html_url": "https://github.com/kaigedong/Manis-Pocket/releases/tag/build-2.7.142",
                "assets": [{
                    "name": MACOS_ASSET_NAME,
                    "browser_download_url": "https://github.com/kaigedong/Manis-Pocket/releases/download/build-2.7.142/ManisPocket.app.zip",
                    "digest": VALID_DIGEST,
                    "size": 12_329_836
                }]
            },
            {
                "tag_name": "v2.8.0",
                "draft": false,
                "html_url": "https://github.com/kaigedong/Manis-Pocket/releases/tag/v2.8.0",
                "assets": [{
                    "name": MACOS_ASSET_NAME,
                    "browser_download_url": "https://github.com/kaigedong/Manis-Pocket/releases/download/v2.8.0/ManisPocket.app.zip",
                    "digest": VALID_DIGEST,
                    "size": 12_500_000
                }]
            }
        ])
        .to_string()
        .into_bytes()
    }

    #[test]
    fn parses_release_and_dev_versions() {
        assert_eq!(
            Version::parse_installed("2.7.142"),
            Ok(Version([2, 7, 142]))
        );
        assert_eq!(
            Version::parse_installed("0.1.0.r1136.gf85a27d"),
            Ok(Version([0, 1, 0]))
        );
        assert_eq!(
            Version::parse_installed("0.1.x"),
            Err(AppUpdateError::InvalidMetadata)
        );
        assert_eq!(
            Version::parse_release_tag("build-2.7.142"),
            Some((Version([2, 7, 142]), "2.7.142".to_owned()))
        );
        assert_eq!(
            Version::parse_release_tag("v2.8.0"),
            Some((Version([2, 8, 0]), "2.8.0".to_owned()))
        );
        assert_eq!(Version::parse_release_tag("latest"), None);
    }

    #[test]
    fn selects_highest_non_draft_release_above_installed() {
        let payload = sample_releases_json();
        let update = select_available_update(&payload, "2.7.141", true)
            .expect("valid metadata")
            .expect("newer version exists");
        assert_eq!(update.version, "2.8.0");
        assert_eq!(update.tag_name, "v2.8.0");
        assert!(update.asset.is_some());

        let up_to_date = select_available_update(&payload, "2.8.0", true).expect("valid metadata");
        assert_eq!(up_to_date, None);
    }

    #[test]
    fn reports_missing_package_when_self_install_asset_is_absent() {
        let payload = serde_json::json!([
            {
                "tag_name": "build-2.7.150",
                "draft": false,
                "html_url": "https://github.com/kaigedong/Manis-Pocket/releases/tag/build-2.7.150",
                "assets": [{
                    "name": "ManisPocket-linux-x86_64.tar.gz",
                    "browser_download_url": "https://github.com/kaigedong/Manis-Pocket/releases/download/build-2.7.150/ManisPocket-linux-x86_64.tar.gz",
                    "digest": VALID_DIGEST,
                    "size": 20_000_000
                }]
            }
        ])
        .to_string()
        .into_bytes();

        assert_eq!(
            select_available_update(&payload, "2.7.142", true),
            Err(AppUpdateError::MissingPackage)
        );
        let linux_update = select_available_update(&payload, "2.7.142", false)
            .expect("non-self-install platforms still notify")
            .expect("update found");
        assert_eq!(linux_update.version, "2.7.150");
        assert!(linux_update.asset.is_none());
    }

    #[test]
    fn verifies_sha256_digest() {
        let payload = b"manis pocket update payload";
        let digest = hex::encode(Sha256::digest(payload));
        assert_eq!(verify_digest(payload, &digest), Ok(()));
        assert_eq!(
            verify_digest(payload, &"0".repeat(64)),
            Err(AppUpdateError::DigestMismatch)
        );
        assert_eq!(parse_github_digest(&format!("sha256:{digest}")), Ok(digest));
        assert_eq!(
            parse_github_digest("md5:1234"),
            Err(AppUpdateError::InvalidDigest)
        );
    }

    #[test]
    fn package_stream_reports_progress_and_enforces_limit() {
        let payload = vec![0x5a_u8; 40_000];
        let mut cursor = std::io::Cursor::new(payload.clone());
        let mut updates = Vec::new();
        let downloaded = read_package_stream(&mut cursor, Some(40_000), 100_000, |progress| {
            updates.push(progress);
        })
        .expect("stream downloads");
        assert_eq!(downloaded, payload);
        assert_eq!(
            updates.first().expect("initial progress").percent(),
            Some(0)
        );
        assert_eq!(updates.last().expect("final progress").percent(), Some(100));

        let mut oversized = std::io::Cursor::new(vec![0_u8; 16]);
        assert_eq!(
            read_package_stream(&mut oversized, Some(16), 8, |_| {}),
            Err(AppUpdateError::PackageTooLarge)
        );
    }

    #[test]
    fn progress_label_matches_manis_formatting() {
        let progress = DownloadProgress {
            downloaded_bytes: 4_200_000,
            total_bytes: Some(10_000_000),
            bytes_per_second: 2_300_000,
        };
        assert_eq!(progress.label(), "正在下载... 42% · 2.2 MB/s");

        let unknown_total = DownloadProgress {
            downloaded_bytes: 512 * 1024,
            total_bytes: None,
            bytes_per_second: 0,
        };
        assert_eq!(unknown_total.label(), "正在下载... 512 KB");
    }

    #[cfg(unix)]
    #[test]
    fn swap_script_waits_and_safely_quotes_spaces() {
        let script = macos_install::swap_script(
            4242,
            Path::new("/Applications/Manis Pocket.app"),
            Path::new("/tmp/manis-pocket-app-update-4242/extract/Manis Pocket.app"),
            Path::new("/tmp/manis-pocket-app-update-4242"),
        );
        assert!(script.contains("kill -0 4242"));
        assert!(script.contains("bundle='/Applications/Manis Pocket.app'"));
        assert!(script.contains("open -n \"$bundle\""));
    }

    #[cfg(unix)]
    #[test]
    fn extracts_bundle_with_spaces_and_skips_macosx_junk() {
        use std::io::Write as _;

        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = zip::write::SimpleFileOptions::default();
            writer
                .start_file("__MACOSX/Manis Pocket.app/._Info.plist", options)
                .expect("write macosx metadata");
            writer.write_all(b"junk").expect("write junk");
            writer
                .start_file("Manis Pocket.app/Contents/Info.plist", options)
                .expect("write plist");
            writer.write_all(b"<plist/>").expect("write plist body");
            writer
                .start_file("Manis Pocket.app/Contents/MacOS/manis-pocket", options)
                .expect("write binary");
            writer.write_all(b"#!/bin/sh\n").expect("write binary body");
            writer.finish().expect("finish zip");
        }

        let staging =
            std::env::temp_dir().join(format!("manis-pocket-zip-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).expect("create staging");

        macos_install::extract_zip(&buffer, &staging).expect("extract zip");
        let bundle = macos_install::find_bundle(&staging).expect("locate app bundle");
        assert_eq!(bundle, staging.join("Manis Pocket.app"));
        assert!(bundle.join("Contents/MacOS/manis-pocket").is_file());

        let _ = std::fs::remove_dir_all(&staging);
    }
}
