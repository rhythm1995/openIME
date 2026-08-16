import { useTranslation } from "react-i18next";
import { Download, ArrowRight } from "lucide-react";
import { RELEASES_URL } from "../links";

export default function DownloadSection() {
  const { t } = useTranslation();
  const meta = t("download.metaItems", { returnObjects: true }) as string[];

  return (
    <section className="section" id="download">
      <div className="container download-inner">
        <div className="section-head center">
          <p className="eyebrow">{t("download.eyebrow")}</p>
          <h2 className="section-title">{t("download.title")}</h2>
        </div>
        <div className="hero-cta">
          <a className="btn btn-filled" href={RELEASES_URL} target="_blank" rel="noreferrer">
            <Download size={15} />
            {t("download.mac")}
          </a>
          <a className="btn btn-ghost" href={RELEASES_URL} target="_blank" rel="noreferrer">
            {t("download.win")}
          </a>
        </div>
        <p className="download-meta meta-mono">
          {meta.map((item, i) => (
            <span key={i}>
              {i > 0 && <span className="sep">|</span>}
              {item}
            </span>
          ))}
        </p>
        <p>
          <a className="text-link" href={RELEASES_URL} target="_blank" rel="noreferrer">
            {t("download.go")}
            <ArrowRight size={14} className="link-arrow" aria-hidden="true" />
          </a>
        </p>
      </div>
    </section>
  );
}
