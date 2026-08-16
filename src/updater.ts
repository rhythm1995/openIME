// 应用内自动更新：封装 tauri updater 插件（更新源见 tauri.conf.json
// plugins.updater.endpoints —— GitHub Releases 的 latest.json）。
//
// 约定：dev 构建 / 无网络 / 更新源不可达等一律静默返回 null / 吞错，
// 不打断用户；下载与安装由用户在 设置 → App 行为 显式触发。
import { check } from "@tauri-apps/plugin-updater";
import type { Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

export type { Update };

/** 当前应用版本（失败返回空串，UI 显示时容忍）。 */
export async function currentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

/** 检查更新：有新版本返回 Update，无更新 / dev / 网络 or 签名异常返回 null。 */
export async function checkUpdate(): Promise<Update | null> {
  try {
    return await check();
  } catch {
    return null;
  }
}

export interface DownloadProgress {
  downloaded: number;
  /** 总大小未知时缺省（渐进显示 MB）。 */
  contentLength?: number;
}

/** 下载并安装（进度回调），完成后由调用方 relaunch。 */
export async function downloadAndInstall(
  update: Update,
  onProgress?: (p: DownloadProgress) => void,
): Promise<void> {
  let downloaded = 0;
  let contentLength: number | undefined;
  await update.downloadAndInstall((event: { event: string; data?: { contentLength?: number; chunkLength?: number } }) => {
    switch (event.event) {
      case "Started":
        contentLength = event.data?.contentLength;
        break;
      case "Progress":
        downloaded += event.data?.chunkLength ?? 0;
        onProgress?.({ downloaded, contentLength });
        break;
      default:
        break;
    }
  });
}

/** 更新安装完成后重启应用。 */
export async function relaunchApp(): Promise<void> {
  await relaunch();
}
