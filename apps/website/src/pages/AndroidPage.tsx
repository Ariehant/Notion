import { site } from "../site.config";
import { Icon } from "../components/icons";
import { Footer } from "../sections/Footer";
import { HeroAndroid } from "../sections/android/HeroAndroid";
import { PillarsAndroid } from "../sections/android/PillarsAndroid";
import { SecurityAndroid } from "../sections/android/SecurityAndroid";
import { DownloadAndroid } from "../sections/android/DownloadAndroid";

/** The /android landing page. Reuses the site's design system and footer. */
export function AndroidPage() {
  return (
    <>
      <header className="nav">
        <div className="container nav-inner">
          <a className="brand" href="#top">
            <img src="/logo.png" alt="" width={28} height={28} className="brand-logo" />
            <span className="brand-name">
              {site.productName} <span className="brand-tag">for Android</span>
            </span>
          </a>
          <nav className="nav-links" aria-label="Primary">
            <a href="#features">Features</a>
            <a href="#security">Security</a>
            <a href="#download">Get it</a>
            <a href="/">Desktop</a>
          </nav>
          <div className="nav-actions">
            <a className="btn btn-ghost" href={site.githubUrl} target="_blank" rel="noreferrer">
              <Icon name="github" size={18} /> GitHub
            </a>
            <a className="btn btn-primary" href="#download">
              <Icon name="download" size={18} /> Get the app
            </a>
          </div>
        </div>
      </header>
      <main>
        <HeroAndroid />
        <PillarsAndroid />
        <SecurityAndroid />
        <DownloadAndroid />
      </main>
      <Footer />
    </>
  );
}
