import { site } from "../site.config";
import { Icon } from "../components/icons";
import { MockWindow } from "../components/MockWindow";
import { trustChips } from "../data/features";

export function Hero() {
  return (
    <section className="hero" id="top">
      <div className="container hero-inner">
        <div className="hero-copy">
          <span className="hero-eyebrow">
            <Icon name="shield" size={14} /> {site.tagline}
          </span>
          <h1 className="hero-title">
            Your notes. <span className="accent">Encrypted.</span>
            <br />
            On your machine.
          </h1>
          <p className="hero-sub">
            A block editor, a native calendar companion, and local AI — all reading one encrypted
            database that never leaves your device. No cloud, no account, no lock-in.
          </p>
          <div className="hero-cta">
            <a className="btn btn-primary btn-lg" href="#download">
              <Icon name="download" size={19} /> Download free
            </a>
            <a
              className="btn btn-ghost btn-lg"
              href={site.githubUrl}
              target="_blank"
              rel="noreferrer"
            >
              <Icon name="github" size={19} /> View source
            </a>
          </div>
          <ul className="hero-chips">
            {trustChips.map((c) => (
              <li key={c.label}>
                <Icon name={c.icon} size={15} /> {c.label}
              </li>
            ))}
          </ul>
        </div>
        <div className="hero-art">
          <MockWindow />
        </div>
      </div>
    </section>
  );
}
