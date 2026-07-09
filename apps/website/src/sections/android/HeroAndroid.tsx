import { Icon } from "../../components/icons";
import { PhoneMock } from "../../components/PhoneMock";
import { site } from "../../site.config";
import { androidTagline, androidTrustChips } from "../../data/android";

export function HeroAndroid() {
  return (
    <section className="hero" id="top">
      <div className="container hero-inner">
        <div className="hero-copy">
          <span className="hero-eyebrow">
            <Icon name="android" size={14} /> {androidTagline}
          </span>
          <h1 className="hero-title">
            Your notes. <span className="accent">Encrypted.</span>
            <br />
            In your pocket.
          </h1>
          <p className="hero-sub">
            An offline-first, open-source notes app for Android 11 and above. Everything is
            encrypted on your device and never touches the cloud — unlock with your fingerprint,
            write in a fast block editor, and see your day from the home screen.
          </p>
          <div className="hero-cta">
            <a className="btn btn-primary btn-lg" href="#download">
              <Icon name="download" size={19} /> Get the app
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
            {androidTrustChips.map((c) => (
              <li key={c.label}>
                <Icon name={c.icon} size={15} /> {c.label}
              </li>
            ))}
          </ul>
        </div>
        <div className="hero-art">
          <PhoneMock />
        </div>
      </div>
    </section>
  );
}
