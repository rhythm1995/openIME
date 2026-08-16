import { useTranslation } from "react-i18next";
import { Download } from "lucide-react";
import GithubIcon from "./GithubIcon";
import WaveDemo from "./WaveDemo";
import { RELEASES_URL, REPO_URL } from "../links";

export default function Hero() {
  const { t } = useTranslation();
  return (
    <section className="hero" id="top">
      <div className="container">
        <h1 className="hero-title">{t("hero.title")}</h1>
        <p className="hero-sub">{t("hero.subtitle")}</p>
        <div className="hero-cta">
          <a className="btn btn-primary" href={RELEASES_URL} target="_blank" rel="noreferrer">
            <Download size={16} />
            {t("hero.ctaPrimary")}
          </a>
          <a className="btn btn-ghost" href={REPO_URL} target="_blank" rel="noreferrer">
            <GithubIcon size={16} />
            {t("hero.ctaSecondary")}
          </a>
        </div>
        <p className="hero-note">{t("hero.platformNote")}</p>
        <WaveDemo />
      </div>
    </section>
  );
}
