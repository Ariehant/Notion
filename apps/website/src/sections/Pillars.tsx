import { Icon } from "../components/icons";
import { pillars } from "../data/features";

export function Pillars() {
  return (
    <section className="section" id="features">
      <div className="container">
        <div className="section-head">
          <span className="eyebrow">Everything, on-device</span>
          <h2 className="section-title">Three tools. One encrypted database.</h2>
          <p className="section-lead">
            Write, see your day, and think with AI — each surface reads the same encrypted file, so
            your data is never duplicated and never leaves your machine.
          </p>
        </div>

        <div className="pillars">
          {pillars.map((p) => (
            <article className="pillar" key={p.title}>
              <div className="pillar-icon">
                <Icon name={p.icon} size={22} />
              </div>
              <span className="eyebrow">{p.eyebrow}</span>
              <h3 className="pillar-title">{p.title}</h3>
              <p className="pillar-summary">{p.summary}</p>
              <ul className="pillar-features">
                {p.features.map((f) => (
                  <li key={f.title}>
                    <Icon name="check" size={16} className="feat-check" />
                    <div>
                      <strong>{f.title}</strong>
                      <span>{f.body}</span>
                    </div>
                  </li>
                ))}
              </ul>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}
