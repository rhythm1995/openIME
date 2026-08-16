import { useTranslation } from "react-i18next";

type Row = { engine: string; deploy: string; note: string };

export default function EngineTable() {
  const { t } = useTranslation();
  const rows = t("engines.rows", { returnObjects: true }) as Row[];

  return (
    <section className="section" id="engines">
      <div className="container">
        <p className="eyebrow">{t("engines.eyebrow")}</p>
        <h2 className="section-title">{t("engines.title")}</h2>
        <div className="table-scroll">
          <table className="spec-table">
            <thead>
              <tr>
                <th>{t("engines.colEngine")}</th>
                <th>{t("engines.colDeploy")}</th>
                <th>{t("engines.colNote")}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.engine}>
                  <td className="mono">{r.engine}</td>
                  <td>
                    <span className={`deploy ${r.deploy === "local" ? "local" : "cloud"}`}>
                      <span className="deploy-dot" aria-hidden="true" />
                      {r.deploy === "local" ? t("engines.deployLocal") : t("engines.deployCloud")}
                    </span>
                  </td>
                  <td className="note">{r.note}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
