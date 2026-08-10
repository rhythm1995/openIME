import { useEffect, useState, type ReactNode } from "react";
import { ipc, permissionLabel, type PermissionKind, type PermissionStatus } from "../ipc";

// 顶部权限横幅：不阻塞使用，缺失权限时提示，可一键授权或忽略。
// - 每项权限独立操作：「授权」触发系统弹窗/请求，「系统设置」深链到对应面板。
// - 权限未齐时每 2.5s 轮询一次（用户在系统设置里勾选后自动消失）。
export default function PermissionBanner() {
  const [mic, setMic] = useState<PermissionStatus | null>(null);
  const [ax, setAx] = useState<PermissionStatus | null>(null);
  const [dismissed, setDismissed] = useState(false);

  // 链式轮询：两项都授权后停止。
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const tick = async () => {
      let m: PermissionStatus | null = null;
      let a: PermissionStatus | null = null;
      try {
        m = await ipc.checkPermission("microphone" as PermissionKind);
      } catch {
        /* 后端未就绪 */
      }
      try {
        a = await ipc.checkPermission("accessibility" as PermissionKind);
      } catch {
        /* 后端未就绪 */
      }
      if (cancelled) return;
      setMic(m);
      setAx(a);
      if (m?.state === "granted" && a?.state === "granted") return;
      timer = setTimeout(tick, 2500);
    };
    tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  const axOk = ax?.state === "granted";
  const micOk = mic?.state === "granted";
  if (dismissed || (axOk && micOk)) return null;

  const item = (label: string, status: PermissionStatus | null, actions: ReactNode) => (
    <div className="row-between" style={{ padding: "4px 0", gap: 8 }}>
      <div style={{ fontSize: 13 }}>
        {label}：{status ? permissionLabel[status.state] : "未知"}
      </div>
      <div style={{ display: "flex", gap: 6 }}>{actions}</div>
    </div>
  );

  return (
    <div className="card" style={{ background: "var(--accent-soft)", boxShadow: "none" }}>
      <div style={{ fontWeight: 600, marginBottom: 6 }}>需要授权才能完整使用</div>
      {!axOk &&
        item(
          "辅助功能",
          ax,
          <>
            <button className="btn btn-sm" onClick={() => ipc.requestAccessibility()}>
              授权
            </button>
            <button
              className="btn btn-sm btn-ghost"
              onClick={() => ipc.openPermissionSettings("accessibility" as PermissionKind)}
            >
              系统设置
            </button>
          </>
        )}
      {!micOk &&
        item(
          "麦克风",
          mic,
          <>
            <button className="btn btn-sm" onClick={() => ipc.requestMicrophone()}>
              授权
            </button>
            <button
              className="btn btn-sm btn-ghost"
              onClick={() => ipc.openPermissionSettings("microphone" as PermissionKind)}
            >
              系统设置
            </button>
          </>
        )}
      <div className="row-between" style={{ marginTop: 6 }}>
        <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
          在系统设置里勾选后，这里会自动更新
        </span>
        <button className="btn btn-sm btn-ghost" onClick={() => setDismissed(true)}>
          稍后
        </button>
      </div>
    </div>
  );
}
