import { useTranslation } from "react-i18next";
import GithubIcon from "./GithubIcon";
import WaveMark from "./WaveMark";
import { DOCS_URL, REPO_URL } from "../links";

export default function Nav() {
  const { t, i18n } = useTranslation();
  const next = i18n.language === "en" ? "zh" : "en";

  return (
    <header className="nav">
      <div className="container nav-inner">
        <a className="brand" href="#top" aria-label="openIME">
          <WaveMark size={24} />
          <span className="brand-word">openIME</span>
        </a>
        <nav className="nav-links" aria-label="site">
          <a href="#features">{t("nav.features")}</a>
          <a href="#privacy">{t("nav.privacy")}</a>
          <a href="#download">{t("nav.download")}</a>
          <a href={DOCS_URL} target="_blank" rel="noreferrer">
            {t("nav.docs")}
          </a>
        </nav>
        <div className="nav-actions">
          <button
            className="lang-btn"
            onClick={() => i18n.changeLanguage(next)}
            aria-label={t("nav.langAria")}
          >
            {next === "en" ? "EN" : "中"}
          </button>
          <a
            className="icon-btn"
            href={REPO_URL}
            target="_blank"
            rel="noreferrer"
            aria-label={t("nav.githubAria")}
          >
            <GithubIcon size={17} />
          </a>
        </div>
      </div>
    </header>
  );
}
