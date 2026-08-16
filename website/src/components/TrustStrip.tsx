import { useTranslation } from "react-i18next";

export default function TrustStrip() {
  const { t } = useTranslation();
  const items = t("trust.items", { returnObjects: true }) as string[];
  return (
    <div className="trust">
      <div className="container trust-inner">
        {items.map((item, i) => (
          <span key={i} className="trust-item">
            {i > 0 && <span className="trust-sep" aria-hidden="true">·</span>}
            {item}
          </span>
        ))}
      </div>
    </div>
  );
}
