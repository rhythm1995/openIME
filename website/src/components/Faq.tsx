import { useTranslation } from "react-i18next";

type Item = { q: string; a: string };

export default function Faq() {
  const { t } = useTranslation();
  const items = t("faq.items", { returnObjects: true }) as Item[];

  return (
    <section className="section" id="faq">
      <div className="container">
        <p className="eyebrow">{t("faq.eyebrow")}</p>
        <div className="faq-list">
          {items.map((item) => (
            <details className="faq-item" key={item.q}>
              <summary>{item.q}</summary>
              <p>{item.a}</p>
            </details>
          ))}
        </div>
      </div>
    </section>
  );
}
