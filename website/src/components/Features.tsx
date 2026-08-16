import { useTranslation } from "react-i18next";
import {
  AudioLines,
  BookMarked,
  FileText,
  Languages,
  Wand2,
} from "lucide-react";

type Feature = { num: string; title: string; desc: string; meta: string };

const FEATURE_ICONS = [AudioLines, Wand2, Languages, BookMarked, FileText];

export default function Features() {
  const { t } = useTranslation();
  const items = t("features.items", { returnObjects: true }) as Feature[];

  return (
    <section className="section" id="features">
      <div className="container">
        <div className="section-head">
          <p className="eyebrow">{t("features.eyebrow")}</p>
          <h2 className="section-title">{t("features.title")}</h2>
        </div>
        <div className="feature-grid">
          {items.map((f, i) => {
            const Icon = FEATURE_ICONS[i] ?? AudioLines;
            return (
              <div className="key-card" key={f.num}>
                <div className="icon-orb">
                  <Icon size={24} strokeWidth={1.8} aria-hidden="true" />
                </div>
                <h3 className="key-card-title">{f.title}</h3>
                <p className="key-card-desc">{f.desc}</p>
                <p className="key-card-meta">{f.meta}</p>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
