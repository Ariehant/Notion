import { site } from "../site.config";
import { Icon } from "../components/icons";
import { downloads } from "../data/features";

export function Download() {
  return (
    <section className="section" id="download">
      <div className="container">
        <div className="section-head">
          <span className="eyebrow">Get it</span>
          <h2 className="section-title">Download and run — no build step.</h2>
          <p className="section-lead">
            Grab the build for your OS from the latest release. Free and open source.
          </p>
        </div>

        <div className="os-grid">
          {downloads.map((d) => (
            <a
              className="os-card"
              key={d.os}
              href={site.releasesUrl}
              target="_blank"
              rel="noreferrer"
            >
              <Icon name={d.icon} size={34} className="os-icon" />
              <strong>{d.os}</strong>
              <code>{d.file}</code>
              <span className="os-note">{d.note}</span>
              <span className="os-cta">
                Download <Icon name="arrowRight" size={15} />
              </span>
            </a>
          ))}
        </div>

        <div className="dl-foot">
          <a
            className="btn btn-primary btn-lg"
            href={site.releasesUrl}
            target="_blank"
            rel="noreferrer"
          >
            <Icon name="download" size={19} /> All releases
          </a>
          <p className="dl-note">
            Prefer to build it yourself?{" "}
            <code>pnpm install &amp;&amp; pnpm --filter @notion/desktop exec tauri build</code>. AI
            features are off by default — launch with <code>ENABLE_OPEN_NOTEBOOK=1</code> to turn
            them on.
          </p>
        </div>
      </div>
    </section>
  );
}
