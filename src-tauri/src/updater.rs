use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// GitHub repo that hosts release artifacts.
/// TODO: confirm name/owner before first publish.
const REPO_API: &str = "https://api.github.com/repos/rmirabelle/playersage/releases/latest";
const USER_AGENT: &str = "PlayerSage-Updater";

#[derive(Debug, Serialize, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
    pub asset_name: String,
    pub release_notes: String,
    pub release_url: String,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn parse_version(s: &str) -> Vec<u32> {
    s.trim_start_matches(|c: char| !c.is_ascii_digit())
        .split('.')
        .take(4)
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .map(|p| p.parse::<u32>().unwrap_or(0))
        .collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    let l = parse_version(latest);
    let c = parse_version(current);
    let n = l.len().max(c.len());
    for i in 0..n {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv != cv {
            return lv > cv;
        }
    }
    false
}

async fn fetch_latest() -> Result<GhRelease> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
    let resp = client
        .get(REPO_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("network error")?;
    if !resp.status().is_success() {
        return Err(anyhow!("github api returned {}", resp.status()));
    }
    let release: GhRelease = resp.json().await.context("malformed release json")?;
    Ok(release)
}

fn pick_installer_asset(assets: &[GhAsset]) -> Option<&GhAsset> {
    assets
        .iter()
        .find(|a| a.name.to_lowercase().ends_with("-setup.exe"))
        .or_else(|| assets.iter().find(|a| a.name.to_lowercase().ends_with(".exe")))
}

#[tauri::command]
pub async fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    let current = current_version().to_string();
    let release = match fetch_latest().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[updater] check failed: {e:#}");
            return Ok(None);
        }
    };
    let latest = release.tag_name.trim_start_matches('v').to_string();
    if !is_newer(&latest, &current) {
        return Ok(None);
    }
    let asset = match pick_installer_asset(&release.assets) {
        Some(a) => a,
        None => {
            eprintln!("[updater] no installer asset on latest release");
            return Ok(None);
        }
    };
    Ok(Some(UpdateInfo {
        current_version: current,
        latest_version: latest,
        download_url: asset.browser_download_url.clone(),
        asset_name: asset.name.clone(),
        release_notes: release.body.unwrap_or_default(),
        release_url: release.html_url,
    }))
}

#[tauri::command]
pub async fn download_and_run_installer(
    app: AppHandle,
    url: String,
    asset_name: String,
) -> Result<(), String> {
    let temp_dir = std::env::temp_dir();
    let sanitized = asset_name.replace(['/', '\\'], "_");
    let target = temp_dir.join(&sanitized);

    download_to(&app, &url, &target)
        .await
        .map_err(|e| format!("download failed: {e:#}"))?;

    launch_detached(&target).map_err(|e| format!("launch failed: {e:#}"))?;

    app.exit(0);
    Ok(())
}

async fn download_to(app: &AppHandle, url: &str, target: &PathBuf) -> Result<()> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("download returned {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);

    let mut file = tokio::fs::File::create(target)
        .await
        .with_context(|| format!("create {}", target.display()))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;

    let _ = app.emit(
        "update-progress",
        serde_json::json!({ "downloaded": 0, "total": total }),
    );

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        use tokio::io::AsyncWriteExt;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded - last_emit > 65_536 {
            last_emit = downloaded;
            let _ = app.emit(
                "update-progress",
                serde_json::json!({ "downloaded": downloaded, "total": total }),
            );
        }
    }

    use tokio::io::AsyncWriteExt;
    file.flush().await?;
    drop(file);

    let _ = app.emit(
        "update-progress",
        serde_json::json!({ "downloaded": downloaded, "total": downloaded }),
    );
    Ok(())
}

#[cfg(windows)]
fn launch_detached(path: &PathBuf) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    std::process::Command::new(path)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .with_context(|| format!("spawn {}", path.display()))?;
    Ok(())
}

#[cfg(not(windows))]
fn launch_detached(path: &PathBuf) -> Result<()> {
    std::process::Command::new(path)
        .spawn()
        .with_context(|| format!("spawn {}", path.display()))?;
    Ok(())
}

#[tauri::command]
pub fn get_app_version() -> String {
    current_version().to_string()
}
