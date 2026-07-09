import { site } from "../../site.config";
import { Icon } from "../../components/icons";
import { androidStores } from "../../data/android";

export function DownloadAndroid() {
  return (
    <section className="section" id="download">
      <div className="container">
        <div className="section-head">
          <span className="eyebrow">Get it</span>
          <h2 className="section-title">For Android 11 and above.</h2>
          <p className="section-lead">
            Free and open source. The Android app is on the way — the desktop app for Windows,
            macOS, and Linux is available today.
          </p>
        </div>

        <div className="os-grid">
          {androidStores.map((s) => (
            <a
              className="os-card"
              key={s.name}
              href={site.releasesUrl}
              target="_blank"
              rel="noreferrer"
            >
              <Icon name={s.icon} size={34} className="os-icon" />
              <strong>{s.name}</strong>
              <code>{s.channel}</code>
              <span className="os-note">{s.note}</span>
              <span className="os-cta">
                Details <Icon name="arrowRight" size={15} />
              </span>
            </a>
          ))}
        </div>

        <div className="dl-foot">
          <a className="btn btn-ghost btn-lg" href="/">
            <Icon name="arrowRight" size={19} /> Desktop version
          </a>
          <p className="dl-note">
            Prefer to build it yourself? See <code>apps/desktop/BUILD_ANDROID.md</code> in the repo
            — the Android target is the same Tauri project as the desktop app.
          </p>
        </div>
      </div>
    </section>
  );
}
