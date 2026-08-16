import { useTranslation } from "react-i18next";
import { Globe, Keyboard, Mic, TextCursorInput } from "lucide-react";

type Step = { num: string; title: string; desc: string };

const STEP_ICONS = [Keyboard, Mic, TextCursorInput];

export default function HowItWorks() {
  const { t } = useTranslation();
  const steps = t("how.steps", { returnObjects: true }) as Step[];

  return (
    <section className="section" id="how">
      <div className="container">
        <div className="section-head center">
          <p className="eyebrow">{t("how.eyebrow")}</p>
          <h2 className="section-title">{t("how.title")}</h2>
        </div>
        <div className="steps">
          {steps.map((step, i) => {
            const Icon = STEP_ICONS[i] ?? Keyboard;
            return (
              <div className="key-card step" key={step.num}>
                <div className="icon-orb">
                  <Icon size={24} strokeWidth={1.8} aria-hidden="true" />
                </div>
                <h3 className="key-card-title">{step.title}</h3>
                <p className="key-card-desc">{step.desc}</p>
                {i === 0 && (
                  <p className="step-kbd">
                    <kbd>Fn</kbd>
                    <span className="kbd-or">/</span>
                    <kbd>
                      <Globe size={12} aria-hidden="true" />
                    </kbd>
                    <span className="kbd-note">{t("how.kbdNote")}</span>
                  </p>
                )}
              </div>
            );
          })}
        </div>
        <figure className="window">
          <div className="window-bar" aria-hidden="true">
            <span className="win-dot" />
            <span className="win-dot" />
            <span className="win-dot" />
          </div>
          <img src="screenshots/settings.png" alt={t("how.shotAlt")} loading="lazy" />
          <figcaption className="window-caption meta-mono">
            {t("how.shotCaption")}
          </figcaption>
        </figure>
      </div>
    </section>
  );
}
