import { useTranslation } from "react-i18next";
import WaveMark from "./WaveMark";
import { DOCS_URL, ISSUES_URL, REPO_URL } from "../links";

export default function Footer() {
  const { t } = useTranslation();
  const meta = t("footer.metaItems", { returnObjects: true }) as string[];

  return (
    <footer className="footer">
      <div className="container footer-inner">
        <div className="footer-brand">
          <span className="footer-mark">
            <WaveMark size={20} />
          </span>
          <div>
            <div className="footer-word">openIME</div>
            <div className="footer-tagline">{t("footer.tagline")}</div>
          </div>
        </div>
        <nav className="footer-links" aria-label="footer">
          <a href={DOCS_URL} target="_blank" rel="noreferrer">
            {t("footer.docs")}
          </a>
          <a href={REPO_URL} target="_blank" rel="noreferrer">
            GitHub
          </a>
          <a href={ISSUES_URL} target="_blank" rel="noreferrer">
            {t("footer.issues")}
          </a>
        </nav>
      </div>
      <div className="container footer-license meta-mono">
        {meta.map((item, i) => (
          <span key={i}>
            {i > 0 && <span className="sep">|</span>}
            {item}
          </span>
        ))}
      </div>
    </footer>
  );
}
