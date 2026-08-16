import { useTranslation } from "react-i18next";

type Feature = { num: string; title: string; desc: string };

export default function Features() {
  const { t } = useTranslation();
  const items = t("features.items", { returnObjects: true }) as Feature[];

  return (
    <section className="section" id="features">
      <div className="container">
        <p className="eyebrow">{t("features.eyebrow")}</p>
        <h2 className="section-title">{t("features.title")}</h2>
        <div className="feature-rows">
          {items.map((f) => (
            <div className="feature-row" key={f.num}>
              <span className="feature-num">{f.num}</span>
              <h3 className="feature-title">{f.title}</h3>
              <p className="feature-desc">{f.desc}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
