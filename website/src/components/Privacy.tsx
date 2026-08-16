import { useTranslation } from "react-i18next";
import { Check } from "lucide-react";

export default function Privacy() {
  const { t } = useTranslation();
  const points = t("privacy.points", { returnObjects: true }) as string[];
  const d = t("privacy.diagram", { returnObjects: true }) as Record<string, string>;

  return (
    <section className="section" id="privacy">
      <div className="container privacy-grid">
        <div>
          <p className="eyebrow">{t("privacy.eyebrow")}</p>
          <h2 className="section-title">{t("privacy.title")}</h2>
          <p className="section-desc">{t("privacy.desc")}</p>
          <ul className="check-list">
            {points.map((p, i) => (
              <li key={i}>
                <Check size={15} className="check-icon" aria-hidden="true" />
                {p}
              </li>
            ))}
          </ul>
        </div>
        <figure className="pipeline">
          <div className="pipeline-scroll">
            <svg viewBox="0 0 640 300" role="img" aria-label={d.caption}>
              <defs>
                <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                  <path d="M 0 1 L 9 5 L 0 9" fill="none" stroke="#636366" strokeWidth="1.4" />
                </marker>
                <marker id="arrow-opt" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                  <path d="M 0 1 L 9 5 L 0 9" fill="none" stroke="#a1a1a6" strokeWidth="1.2" strokeDasharray="2 2" />
                </marker>
              </defs>

              {/* 本机边界 */}
              <rect x="10" y="44" width="474" height="216" rx="16" fill="rgba(31,31,33,0.5)" stroke="#2c2c2e" strokeDasharray="5 5" />
              <text x="30" y="70" fill="#a1a1a6" fontSize="12" letterSpacing="1">
                {d.zone}
              </text>

              {/* 麦克风 */}
              <rect x="34" y="118" width="92" height="64" rx="10" fill="#1f1f21" stroke="#2c2c2e" />
              <g stroke="#a1a1a6" strokeWidth="1.6" fill="none">
                <rect x="72" y="130" width="16" height="26" rx="8" />
                <path d="M 64 143 a 16 16 0 0 0 32 0" />
                <line x1="80" y1="159" x2="80" y2="167" />
              </g>
              <text x="80" y="176" textAnchor="middle" fill="#f5f5f7" fontSize="11">
                {d.mic}
              </text>

              {/* 箭头：麦克风 → ASR */}
              <line x1="126" y1="150" x2="172" y2="150" stroke="#636366" strokeWidth="1.4" markerEnd="url(#arrow)" />

              {/* sherpa-onnx */}
              <rect x="176" y="118" width="168" height="64" rx="10" fill="#1f1f21" stroke="#2c2c2e" />
              <text x="260" y="144" textAnchor="middle" fill="#f5f5f7" fontSize="13" className="svg-mono">
                {d.asr}
              </text>
              <text x="260" y="163" textAnchor="middle" fill="#a1a1a6" fontSize="11">
                {d.asrSub}
              </text>

              {/* 箭头：ASR → 上屏 */}
              <line x1="344" y1="150" x2="386" y2="150" stroke="#636366" strokeWidth="1.4" markerEnd="url(#arrow)" />

              {/* 上屏 */}
              <rect x="390" y="118" width="82" height="64" rx="10" fill="#1f1f21" stroke="#2c2c2e" />
              <line x1="404" y1="150" x2="446" y2="150" stroke="#5C6AFF" strokeWidth="1.8" />
              <line x1="446" y1="142" x2="446" y2="158" stroke="#5C6AFF" strokeWidth="1.8" className="svg-caret" />
              <text x="431" y="176" textAnchor="middle" fill="#a1a1a6" fontSize="11">
                {d.out}
              </text>

              {/* 云端（可选）：从本机边界外侧虚线引出 */}
              <rect x="520" y="118" width="104" height="64" rx="10" fill="#161617" stroke="#2c2c2e" strokeDasharray="4 4" />
              <text x="572" y="144" textAnchor="middle" fill="#a1a1a6" fontSize="12">
                {d.cloud}
              </text>
              <text x="572" y="161" textAnchor="middle" fill="#636366" fontSize="10">
                {d.cloudSub}
              </text>
              <path d="M 484 118 C 500 88 510 88 520 118" fill="none" stroke="#a1a1a6" strokeWidth="1.2" strokeDasharray="3 3" markerEnd="url(#arrow-opt)" />
              <text x="506" y="82" textAnchor="middle" fill="#636366" fontSize="11">
                {d.optional}
              </text>
            </svg>
          </div>
          <figcaption className="pipeline-caption">{d.caption}</figcaption>
        </figure>
      </div>
    </section>
  );
}
