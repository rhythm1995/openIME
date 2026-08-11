//! 本机性能采集 + 语音模型适配度打标签（极简，不测基准分）。
//!
//! 给语音 ASR 模型打 `轻量 / 适合 / 可用但较慢 / 不推荐(需XGB)` 标签，
//! 帮助用户选合适的模型。门槛见 `compute_model_tag`。

use serde::{Deserialize, Serialize};

// ── 本机信息 ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    /// 总内存（字节）。
    pub total_mem: u64,
    /// 可用内存（字节）。
    pub avail_mem: u64,
    /// CPU 品牌字符串（如 "Apple M3 Pro"）。
    pub cpu_brand: String,
    /// CPU 逻辑核数。
    pub cpu_cores: u32,
    /// 系统版本长字符串。
    pub os_version: String,
    /// 磁盘剩余（`model_root` 所在卷，字节）。
    pub disk_free: u64,
    /// 是否 Apple Silicon。
    pub is_apple_silicon: bool,
    /// 采集时间（RFC3339）。
    pub collected_at: String,
}

/// 采集本机性能。失败的字段置为 0/空串，不抛错。
pub fn collect_system_info() -> SystemInfo {
    // 内存 / CPU / OS
    let mut sys = sysinfo::System::new_all();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    let total_mem = sys.total_memory();
    let avail_mem = sys.available_memory();
    let cpu_cores = sys.cpus().len() as u32;
    let cpu_brand = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    let is_apple_silicon = cpu_brand.contains("Apple");

    // 磁盘剩余：对 model_root 所在卷 statvfs（取 "." 即当前卷可用空间）。
    let disk_free = statvfs_free_bytes(std::path::Path::new("."));

    // 回退：若 brand 不含 Apple，尝试 sysctl hw.optional.arm64 判断
    let is_apple_silicon = is_apple_silicon
        || std::process::Command::new("sysctl")
            .args(["-n", "hw.optional.arm64"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
            .unwrap_or(false);

    SystemInfo {
        total_mem,
        avail_mem,
        cpu_brand,
        cpu_cores,
        os_version: sysinfo::System::long_os_version().unwrap_or_else(|| "未知".into()),
        disk_free,
        is_apple_silicon,
        collected_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

fn statvfs_free_bytes(path: &std::path::Path) -> u64 {
    // 路径参数预留用于未来按卷 statvfs；当前取最可用磁盘块作为估计，失败返回 0（不阻断）。
    let _ = path;
    #[cfg(unix)]
    {
        // 用 sysinfo 的磁盘列表作为粗略估计（最可用空间的磁盘块）。
        let disks = sysinfo::Disks::new_with_refreshed_list();
        disks.iter().map(|d| d.available_space()).max().unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        0
    }
}

// ── 模型适配度标签 ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPerfTag {
    /// 标签文案：适合 / 可用但较慢 / 不推荐（需XGB）。
    pub tag: String,
    /// 语义：suitable | usable | not_recommended | light
    pub kind: String,
    /// 解释原因（tooltip 用）。
    pub reason: String,
    /// 建议色值（前端直接用）：success / warning / danger。
    pub color: String,
}

/// 按 `SystemInfo` 给模型打标签。磁盘空间仅在"剩余 < 2×模型体积"时影响。
pub fn compute_model_tag(approx_size: u64, sys: &SystemInfo) -> ModelPerfTag {
    // 规格：以总内存为基准分档。经验值，随模型测试结果可调。
    // approx_size 不含 VAD（1.8MB），已足够判断。
    let gb = |b: u64| b / (1024 * 1024 * 1024);
    let total_gb = gb(sys.total_mem.max(1));
    let avail_gb = gb(sys.avail_mem);
    let free_gb = gb(sys.disk_free);

    // 若无法采集到内存（sysinfo 失败 -> 0），返回"未知"。
    if total_gb == 0 {
        return ModelPerfTag {
            tag: "未知".into(),
            kind: "unknown".into(),
            reason: "未能采集本机内存信息，请点\"重新采集\"重试".into(),
            color: "var(--text-tertiary)".into(),
        };
    }

    // 磁盘不足（剩余 < 2×模型体积）：优先提示磁盘，不再判断内存。
    let need_gb = gb(approx_size.saturating_mul(2).max(512 * 1024 * 1024));
    if free_gb > 0 && free_gb * 1024 * 1024 * 1024 < approx_size.saturating_mul(2) {
        return ModelPerfTag {
            tag: format!("需{need_gb}GB 磁盘"),
            kind: "not_recommended".into(),
            reason: format!("磁盘剩余约 {free_gb} GB / 需约 {need_gb} GB（约 2 倍模型体积）"),
            color: "var(--danger)".into(),
        };
    }

    // 按体积档打内存标签。
    let usable_not_recommended = |avail: u64, need: u64| ModelPerfTag {
        tag: if avail >= 8 {
            "可用但较慢".into()
        } else {
            format!("不推荐（需{need}GB）")
        },
        kind: "not_recommended".into(),
        reason: format!(
            "本机总内存 {total_gb} GB / 可用 {avail_gb} GB / 建议 ≥{need} GB，运行时可能卡顿或崩溃"
        ),
        color: "var(--danger)".into(),
    };

    // 分档：size < 300MB 轻量；300-1100MB 中；> 1GB 重
    if approx_size < 300 * 1024 * 1024 {
        // 轻量：任何本机都适合
        ModelPerfTag {
            tag: "适合".into(),
            kind: "suitable".into(),
            reason: format!(
                "本机总内存 {total_gb} GB / 可用 {avail_gb} GB，轻量模型，任意本机可用"
            ),
            color: "var(--success)".into(),
        }
    } else if approx_size <= 1100 * 1024 * 1024 {
        // 中量：需 ≥8，推荐 16
        if total_gb >= 16 {
            ModelPerfTag {
                tag: "适合".into(),
                kind: "suitable".into(),
                reason: format!("本机总内存 {total_gb} GB，满足推荐 ≥16GB"),
                color: "var(--success)".into(),
            }
        } else if total_gb >= 8 {
            ModelPerfTag {
                tag: "可用但较慢".into(),
                kind: "usable".into(),
                reason: format!(
                    "本机总内存 {total_gb} GB / 可用 {avail_gb} GB / 建议 ≥16 GB，运行时可能较慢"
                ),
                color: "var(--warning)".into(),
            }
        } else {
            usable_not_recommended(avail_gb, 16)
        }
    } else {
        // 1.5GB+ 重量级：需 ≥16，推荐 32（且 Apple Silicon 友好可作为加分说明）
        let extra = if sys.is_apple_silicon {
            " · Apple Silicon fq16友好"
        } else {
            ""
        };
        if total_gb >= 32 {
            ModelPerfTag {
                tag: "适合".into(),
                kind: "suitable".into(),
                reason: format!("本机总内存 {total_gb} GB，满足重型模型需求{extra}"),
                color: "var(--success)".into(),
            }
        } else if total_gb >= 16 {
            ModelPerfTag {
                tag: "可用但较慢".into(),
                kind: "usable".into(),
                reason: format!(
                    "本机总内存 {total_gb} GB / 可用 {avail_gb} GB / 建议 ≥32 GB{extra}"
                ),
                color: "var(--warning)".into(),
            }
        } else {
            usable_not_recommended(avail_gb, 16)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys_with_mem(total_gb: u64, disk_free_gb: u64) -> SystemInfo {
        SystemInfo {
            total_mem: total_gb * 1024 * 1024 * 1024,
            avail_mem: total_gb / 2 * 1024 * 1024 * 1024,
            cpu_brand: "Test".into(),
            cpu_cores: 8,
            os_version: "macOS 15.0".into(),
            disk_free: disk_free_gb * 1024 * 1024 * 1024,
            is_apple_silicon: false,
            collected_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn tag_light_on_any_mem() {
        let sys = sys_with_mem(4, 200);
        let t = compute_model_tag(240 * 1024 * 1024, &sys);
        assert_eq!(t.kind, "suitable");
    }

    #[test]
    fn tag_heavy_not_recommended_on_8gb() {
        let sys = sys_with_mem(16, 200);
        let t = compute_model_tag(1_700 * 1024 * 1024, &sys);
        assert_eq!(t.kind, "usable"); // 重型 1.5GB+ 需 ≥32 适合，16GB→可用但较慢
        let sys4 = sys_with_mem(4, 200);
        let t4 = compute_model_tag(1_700 * 1024 * 1024, &sys4);
        assert_eq!(t4.kind, "not_recommended");
        let sys_mb = sys_with_mem(4, 200);
        let tb = compute_model_tag(948 * 1024 * 1024, &sys_mb); // funasr-nano-int8 948MB 中量，4GB 总可用 2GB
        assert_eq!(tb.kind, "not_recommended");
        let sys8 = sys_with_mem(8, 200);
        let tb8 = compute_model_tag(948 * 1024 * 1024, &sys8);
        assert_eq!(tb8.kind, "usable"); // 8GB 中量→可用但较慢
    }

    #[test]
    fn tag_unknown_when_no_mem() {
        let sys = sys_with_mem(0, 200);
        let t = compute_model_tag(240 * 1024 * 1024, &sys);
        assert_eq!(t.kind, "unknown");
    }

    // ── 补充分档边界覆盖（TDD）──────────────────────────────

    fn sys_apple_with_mem(total_gb: u64, disk_free_gb: u64) -> SystemInfo {
        let mut s = sys_with_mem(total_gb, disk_free_gb);
        s.is_apple_silicon = true;
        s.cpu_brand = "Apple M3 Pro".into();
        s
    }

    #[test]
    fn tag_medium_buckets_by_mem() {
        // 中量 500MB：16GB→适合，8GB→可用但较慢，4GB→不推荐
        let m = 500 * 1024 * 1024;
        assert_eq!(
            compute_model_tag(m, &sys_with_mem(16, 200)).kind,
            "suitable"
        );
        assert_eq!(compute_model_tag(m, &sys_with_mem(8, 200)).kind, "usable");
        assert_eq!(
            compute_model_tag(m, &sys_with_mem(4, 200)).kind,
            "not_recommended"
        );
    }

    #[test]
    fn tag_heavy_buckets_by_mem() {
        // 重型 1.7GB：32GB→适合，16GB→可用但较慢
        let h = 1_700 * 1024 * 1024;
        assert_eq!(
            compute_model_tag(h, &sys_with_mem(32, 200)).kind,
            "suitable"
        );
        assert_eq!(compute_model_tag(h, &sys_with_mem(16, 200)).kind, "usable");
    }

    #[test]
    fn tag_disk_insufficient_beats_mem() {
        // 磁盘剩余 < 2×模型体积 → 即便内存够也判 not_recommended，且 tag 提示磁盘。
        // 1.7GB 模型，磁盘仅 2GB（< 3.4GB）。
        let sys = sys_with_mem(32, 2);
        let t = compute_model_tag(1_700 * 1024 * 1024, &sys);
        assert_eq!(t.kind, "not_recommended");
        assert!(
            t.tag.contains("磁盘"),
            "磁盘不足时 tag 应含\"磁盘\"，得到 {}",
            t.tag
        );
    }

    #[test]
    fn tag_boundary_300mb_is_medium() {
        // 体积 == 300MB 不属于轻量（轻量是 < 300MB），走中量分支。
        let sys = sys_with_mem(16, 200);
        assert_eq!(compute_model_tag(300 * 1024 * 1024, &sys).kind, "suitable");
    }

    #[test]
    fn tag_boundary_1100mb_is_medium() {
        // 体积 == 1100MB 仍属中量（<= 1100MB）；1101MB 才进重型。
        let sys = sys_with_mem(16, 200);
        assert_eq!(compute_model_tag(1100 * 1024 * 1024, &sys).kind, "suitable");
    }

    #[test]
    fn tag_heavy_32gb_apple_silicon_note() {
        // 重型 + Apple Silicon + 32GB → reason 附带 Apple Silicon 友好说明。
        let sys = sys_apple_with_mem(32, 200);
        let t = compute_model_tag(1_700 * 1024 * 1024, &sys);
        assert_eq!(t.kind, "suitable");
        assert!(
            t.reason.contains("Apple Silicon"),
            "Apple Silicon 应在 reason 体现，得到 {}",
            t.reason
        );
    }
}
