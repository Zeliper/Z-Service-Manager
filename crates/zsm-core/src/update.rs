use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST_API: &str = "https://api.github.com/repos/Zeliper/Z-Service-Manager/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/Zeliper/Z-Service-Manager/releases/latest";
/// Overrides the release API URL (used to exercise the update flow against a local server).
const API_ENV: &str = "ZSM_UPDATE_API";
const CHECKSUMS: &str = "SHA256SUMS.txt";
pub const INSTALLER_ARGS: [&str; 4] = [
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/ZSMUPDATE",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub page_url: String,
    pub installer: Asset,
    pub checksums: Asset,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent(concat!("ZServiceManager/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn api_url() -> String {
    std::env::var(API_ENV).unwrap_or_else(|_| LATEST_API.to_owned())
}

/// The latest published release if it is newer than `current`.
pub fn check(current: &str) -> Result<Option<Release>, String> {
    let current = Version::parse(current).map_err(|e| format!("현재 버전 해석 실패: {e}"))?;
    let api: ApiRelease = agent()
        .get(&api_url())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("릴리스 조회 실패: {e}"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("릴리스 정보 해석 실패: {e}"))?;
    newer_release(api, &current)
}

fn newer_release(api: ApiRelease, current: &Version) -> Result<Option<Release>, String> {
    if api.draft || api.prerelease {
        return Ok(None);
    }
    let version = Version::parse(api.tag_name.trim_start_matches('v'))
        .map_err(|e| format!("태그 `{}` 해석 실패: {e}", api.tag_name))?;
    if !version.pre.is_empty() || version <= *current {
        return Ok(None);
    }
    let installer_name = format!("ZServiceManager-Setup-{version}.exe");
    let find = |name: &str| {
        api.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| Asset {
                name: a.name.clone(),
                url: a.browser_download_url.clone(),
            })
            .ok_or_else(|| format!("릴리스에 `{name}` 파일이 없음"))
    };
    Ok(Some(Release {
        installer: find(&installer_name)?,
        checksums: find(CHECKSUMS)?,
        page_url: api.html_url,
        version,
    }))
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let resp = agent()
        .get(url)
        .call()
        .map_err(|e| format!("다운로드 실패 {url}: {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut file = File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    io::copy(&mut reader, &mut file).map_err(|e| format!("다운로드 중단 {url}: {e}"))?;
    file.flush().map_err(|e| e.to_string())
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Hash for `name` in `sha256sum` format (`<hex>  <name>` or `<hex> *<name>`).
pub fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

/// Downloads the installer and checksum list into `dir` and verifies the installer.
pub fn download_verified(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let sums_path = dir.join(&release.checksums.name);
    download(&release.checksums.url, &sums_path)?;
    let sums = std::fs::read_to_string(&sums_path).map_err(|e| e.to_string())?;
    let expected = expected_hash(&sums, &release.installer.name)
        .ok_or_else(|| format!("{CHECKSUMS} 에 {} 항목이 없음", release.installer.name))?;
    let installer = dir.join(&release.installer.name);
    download(&release.installer.url, &installer)?;
    let actual = sha256_file(&installer).map_err(|e| e.to_string())?;
    if actual != expected {
        let _ = std::fs::remove_file(&installer);
        return Err(format!("해시 불일치: 기대 {expected}, 실제 {actual}"));
    }
    Ok(installer)
}

/// True when running from an Inno Setup install (its uninstaller sits next to the exe).
pub fn is_installed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("unins000.exe")))
        .is_some_and(|u| u.is_file())
}

pub fn write_resume(path: &Path, ids: &[String]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string(ids)?)
}

/// Reads and deletes the resume list; missing or broken files yield an empty list.
pub fn take_resume(path: &Path) -> Vec<String> {
    let ids = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let _ = std::fs::remove_file(path);
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(tag: &str, prerelease: bool, assets: &[&str]) -> ApiRelease {
        ApiRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            html_url: "https://example/r".into(),
            assets: assets
                .iter()
                .map(|n| ApiAsset {
                    name: n.to_string(),
                    browser_download_url: format!("https://example/{n}"),
                })
                .collect(),
        }
    }

    #[test]
    fn picks_newer_stable_release_only() {
        let cur = Version::parse("0.1.0").unwrap();
        let assets = ["ZServiceManager-Setup-0.1.1.exe", "SHA256SUMS.txt"];
        let r = newer_release(api("v0.1.1", false, &assets), &cur)
            .unwrap()
            .unwrap();
        assert_eq!(r.version, Version::parse("0.1.1").unwrap());
        assert_eq!(
            r.installer.url,
            "https://example/ZServiceManager-Setup-0.1.1.exe"
        );
        assert_eq!(
            newer_release(api("v0.1.0", false, &assets), &cur).unwrap(),
            None
        );
        assert_eq!(
            newer_release(api("v0.1.1", true, &assets), &cur).unwrap(),
            None
        );
        assert_eq!(
            newer_release(api("v0.2.0-rc.1", false, &assets), &cur).unwrap(),
            None
        );
        assert!(newer_release(api("v0.1.1", false, &["SHA256SUMS.txt"]), &cur).is_err());
    }

    #[test]
    fn checksum_lookup() {
        let sums = "ABC123  ZServiceManager-Setup-0.1.1.exe\ndef456 *zsm-0.1.1.zip\n";
        assert_eq!(
            expected_hash(sums, "ZServiceManager-Setup-0.1.1.exe").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            expected_hash(sums, "zsm-0.1.1.zip").as_deref(),
            Some("def456")
        );
        assert_eq!(expected_hash(sums, "other"), None);
    }

    #[test]
    fn resume_round_trip() {
        let path = std::env::temp_dir().join(format!("zsm-resume-{}.json", std::process::id()));
        write_resume(&path, &["a".into(), "b".into()]).unwrap();
        assert_eq!(take_resume(&path), ["a", "b"]);
        assert!(!path.exists());
        assert!(take_resume(&path).is_empty());
    }
}
