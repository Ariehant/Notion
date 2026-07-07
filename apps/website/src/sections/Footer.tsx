import { site } from "../site.config";
import { Icon } from "../components/icons";

export function Footer() {
  return (
    <footer className="footer">
      <div className="container footer-inner">
        <div className="footer-brand">
          <img src="/logo.png" alt="" width={24} height={24} />
          <span>
            {site.productName} <span className="muted">v{site.version}</span>
          </span>
        </div>
        <nav className="footer-links" aria-label="Footer">
          <a href="#features">Features</a>
          <a href="#security">Security</a>
          <a href="#download">Download</a>
          <a href={site.docsUrl} target="_blank" rel="noreferrer">
            Docs
          </a>
          <a href={site.githubUrl} target="_blank" rel="noreferrer">
            <Icon name="github" size={16} /> GitHub
          </a>
        </nav>
      </div>
      <div className="container footer-fine">
        <p>Open source · encrypted on-device · no telemetry.</p>
        <p className="muted">
          Not affiliated with, or endorsed by, Notion Labs, Inc. “Notion” is used here only as the
          working project name.
        </p>
      </div>
    </footer>
  );
}
