import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, MicOff } from "lucide-react";

const BAR_COUNT = 36;

type Phase = "idle" | "recording" | "typing";

/** 品牌声波静止形态：对称包络，中峰最高（呼应 logo）。 */
function idleAmp(i: number): number {
  const c = (BAR_COUNT - 1) / 2;
  const d = Math.abs(i - c) / c;
  return 0.14 + 0.86 * Math.pow(1 - d, 2.4);
}

/** 模拟说话波形：中心游走包络 + 多层正弦。 */
function simulateAmp(t: number, i: number): number {
  const c = (BAR_COUNT - 1) / 2;
  const d = Math.abs(i - c) / c;
  const env = 0.3 + 0.7 * Math.pow(1 - d, 1.4);
  const w =
    Math.abs(Math.sin(t * 7.3 + i * 1.9) * 0.6 + Math.sin(t * 12.7 + i * 3.1) * 0.4);
  return Math.max(0.05, Math.min(1, env * (0.35 + 0.65 * w)));
}

/** 麦克风 RMS（0..1）驱动每根竖条，保留包络与伪随机起伏。 */
function micAmp(rms: number, t: number, i: number): number {
  const c = (BAR_COUNT - 1) / 2;
  const d = Math.abs(i - c) / c;
  const env = 0.3 + 0.7 * Math.pow(1 - d, 1.4);
  const w = Math.abs(Math.sin(t * 9.1 + i * 2.3));
  return Math.max(0.05, Math.min(1, env * (rms * 2.2) * (0.55 + 0.45 * w)));
}

export default function WaveDemo() {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>("idle");
  const [typed, setTyped] = useState("");
  const [micOn, setMicOn] = useState(false);
  const [micDenied, setMicDenied] = useState(false);

  const panelRef = useRef<HTMLDivElement>(null);
  const barsRef = useRef<(HTMLDivElement | null)[]>([]);
  const phaseRef = useRef<Phase>("idle");
  const rafRef = useRef(0);
  const startRef = useRef(0);
  const typerRef = useRef(0);
  const resetRef = useRef(0);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const micDataRef = useRef<Uint8Array<ArrayBuffer> | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const micOnRef = useRef(false);

  const setBar = (i: number, amp: number, animate: boolean) => {
    const el = barsRef.current[i];
    if (!el) return;
    el.style.transition = animate ? "transform 0.5s ease" : "none";
    el.style.transform = `scaleY(${amp})`;
  };

  const resetBars = useCallback(() => {
    for (let i = 0; i < BAR_COUNT; i++) setBar(i, idleAmp(i), true);
  }, []);

  const readRms = () => {
    const analyser = analyserRef.current;
    const data = micDataRef.current;
    if (!analyser || !data) return 0;
    analyser.getByteTimeDomainData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i++) {
      const v = (data[i] - 128) / 128;
      sum += v * v;
    }
    return Math.sqrt(sum / data.length);
  };

  const stop = useCallback(() => {
    if (phaseRef.current !== "recording") return;
    cancelAnimationFrame(rafRef.current);
    phaseRef.current = "typing";
    setPhase("typing");

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const text = t("hero.demoText");
    if (reduced) {
      setTyped(text);
      resetRef.current = window.setTimeout(() => {
        phaseRef.current = "idle";
        setPhase("idle");
        setTyped("");
        resetBars();
      }, 3200);
      return;
    }

    let i = 0;
    const tick = () => {
      i += 1;
      setTyped(text.slice(0, i));
      if (i < text.length) {
        typerRef.current = window.setTimeout(tick, 90);
      } else {
        resetRef.current = window.setTimeout(() => {
          phaseRef.current = "idle";
          setPhase("idle");
          setTyped("");
          resetBars();
        }, 3200);
      }
    };
    typerRef.current = window.setTimeout(tick, 160);
  }, [t, resetBars]);

  const start = useCallback(() => {
    if (phaseRef.current === "recording") return;
    window.clearTimeout(typerRef.current);
    window.clearTimeout(resetRef.current);
    setTyped("");
    phaseRef.current = "recording";
    setPhase("recording");
    startRef.current = performance.now();

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) {
      for (let i = 0; i < BAR_COUNT; i++) setBar(i, simulateAmp(0.4, i), true);
      return;
    }

    const loop = () => {
      const t2 = (performance.now() - startRef.current) / 1000;
      if (micOnRef.current && analyserRef.current) {
        const rms = readRms();
        for (let i = 0; i < BAR_COUNT; i++) setBar(i, micAmp(rms, t2, i), false);
      } else {
        for (let i = 0; i < BAR_COUNT; i++) setBar(i, simulateAmp(t2, i), false);
      }
      if (phaseRef.current === "recording") rafRef.current = requestAnimationFrame(loop);
    };
    rafRef.current = requestAnimationFrame(loop);
  }, []);

  const teardownMic = useCallback(() => {
    streamRef.current?.getTracks().forEach((tr) => tr.stop());
    streamRef.current = null;
    analyserRef.current = null;
    micDataRef.current = null;
    audioCtxRef.current?.close().catch(() => undefined);
    audioCtxRef.current = null;
    micOnRef.current = false;
    setMicOn(false);
  }, []);

  const enableMic = useCallback(async () => {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const Ctx: typeof AudioContext =
        window.AudioContext ??
        (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      const ctx = new Ctx();
      const src = ctx.createMediaStreamSource(stream);
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      src.connect(analyser);
      streamRef.current = stream;
      audioCtxRef.current = ctx;
      analyserRef.current = analyser;
      micDataRef.current = new Uint8Array(analyser.fftSize);
      micOnRef.current = true;
      setMicOn(true);
      setMicDenied(false);
    } catch {
      setMicDenied(true);
    }
  }, []);

  useEffect(() => {
    resetBars();
    const onBlur = () => stop();
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("blur", onBlur);
      cancelAnimationFrame(rafRef.current);
      window.clearTimeout(typerRef.current);
      window.clearTimeout(resetRef.current);
      teardownMic();
    };
  }, [resetBars, stop, teardownMic]);

  return (
    <div className="demo-wrap">
      <div
        ref={panelRef}
        className={`demo-panel ${phase}`}
        role="group"
        aria-label={t("hero.demoAriaLabel")}
        tabIndex={0}
        onPointerDown={(e) => {
          e.preventDefault();
          try {
            panelRef.current?.setPointerCapture?.(e.pointerId);
          } catch {
            /* 合成事件无有效 pointerId 时忽略，不影响 start */
          }
          start();
        }}
        onPointerUp={stop}
        onPointerCancel={stop}
        onKeyDown={(e) => {
          if (e.key === " " && !e.repeat) {
            e.preventDefault();
            start();
          }
        }}
        onKeyUp={(e) => {
          if (e.key === " ") stop();
        }}
      >
        <div className="demo-status">
          <span className={`demo-dot ${phase === "recording" ? "live" : ""}`} />
          <span>{phase === "recording" ? t("hero.demoRecording") : t("hero.demoHint")}</span>
        </div>
        <div className="demo-bars" aria-hidden="true">
          {Array.from({ length: BAR_COUNT }, (_, i) => (
            <div
              key={i}
              className="demo-bar"
              ref={(el) => {
                barsRef.current[i] = el;
              }}
            />
          ))}
        </div>
        <div className="demo-result" aria-live="polite">
          <span className="demo-text">{typed}</span>
          <span className="demo-caret" aria-hidden="true" />
        </div>
      </div>
      <div className="demo-foot">
        <button
          className="mic-toggle"
          onClick={() => (micOn ? teardownMic() : enableMic())}
          aria-pressed={micOn}
        >
          {micOn ? <Mic size={13} /> : <MicOff size={13} />}
          <span>{t("hero.micToggle")}</span>
        </button>
        {micDenied && <span className="mic-denied">{t("hero.micDenied")}</span>}
      </div>
    </div>
  );
}
