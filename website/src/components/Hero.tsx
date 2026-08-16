import { useTranslation } from "react-i18next";
import { Download } from "lucide-react";
import GithubIcon from "./GithubIcon";
import WaveDemo from "./WaveDemo";
import { RELEASES_URL, REPO_URL } from "../links";

export default function Hero() {
  const { t } = useTranslation();
  const meta = t("hero.metaItems", { returnObjects: true }) as string[];

  return (
    <section className="hero" id="top">
      <div className="hero-art" aria-hidden="true">
        <span className="bar bar-a" />
        <span className="bar bar-b" />
      </div>
      <div className="container hero-inner">
        <p className="hero-eyebrow">{t("hero.eyebrow")}</p>
        <h1 className="hero-title">{t("hero.title")}</h1>
        <p className="hero-sub">{t("hero.subtitle")}</p>
        <div className="hero-cta">
          <a className="btn btn-filled" href={RELEASES_URL} target="_blank" rel="noreferrer">
            <Download size={15} />
            {t("hero.ctaPrimary")}
          </a>
          <a className="btn btn-ghost" href={REPO_URL} target="_blank" rel="noreferrer">
            <GithubIcon size={15} />
            {t("hero.ctaSecondary")}
          </a>
        </div>
        <p className="hero-meta meta-mono">
          {meta.map((item, i) => (
            <span key={i}>
              {i > 0 && <span className="sep">|</span>}
              {item}
            </span>
          ))}
        </p>
        <WaveDemo />
      </div>
    </section>
  );
}
