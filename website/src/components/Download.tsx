import { useTranslation } from "react-i18next";
import { Download, ArrowRight } from "lucide-react";
import { RELEASES_URL } from "../links";

export default function DownloadSection() {
  const { t } = useTranslation();
  return (
    <section className="section" id="download">
      <div className="container download-inner">
        <p className="eyebrow">{t("download.eyebrow")}</p>
        <h2 className="section-title">{t("download.title")}</h2>
        <div className="hero-cta">
          <a className="btn btn-primary" href={RELEASES_URL} target="_blank" rel="noreferrer">
            <Download size={16} />
            {t("download.mac")}
          </a>
          <a className="btn btn-ghost" href={RELEASES_URL} target="_blank" rel="noreferrer">
            {t("download.win")}
          </a>
        </div>
        <p className="download-note">{t("download.note")}</p>
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
