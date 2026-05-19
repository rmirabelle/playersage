import { invoke } from "@tauri-apps/api/core";

export type UpdateInfo = {
  currentVersion: string;
  latestVersion: string;
  downloadUrl: string;
  assetName: string;
  releaseNotes: string;
  releaseUrl: string;
};

type RawUpdateInfo = {
  current_version: string;
  latest_version: string;
  download_url: string;
  asset_name: string;
  release_notes: string;
  release_url: string;
};

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  const raw = await invoke<RawUpdateInfo | null>("check_for_update");
  if (!raw) return null;
  return {
    currentVersion: raw.current_version,
    latestVersion: raw.latest_version,
    downloadUrl: raw.download_url,
    assetName: raw.asset_name,
    releaseNotes: raw.release_notes,
    releaseUrl: raw.release_url,
  };
}
