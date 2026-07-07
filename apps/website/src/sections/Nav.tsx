import { site } from "../site.config";
import { Icon } from "../components/icons";

export function Nav() {
  return (
    <header className="nav">
      <div className="container nav-inner">
        <a className="brand" href="#top">
          <img src="/logo.png" alt="" width={28} height={28} className="brand-logo" />
          <span className="brand-name">{site.productName}</span>
        </a>
        <nav className="nav-links" aria-label="Primary">
          <a href="#features">Features</a>
          <a href="#security">Security</a>
          <a href="#download">Download</a>
        </nav>
        <div className="nav-actions">
          <a className="btn btn-ghost" href={site.githubUrl} target="_blank" rel="noreferrer">
            <Icon name="github" size={18} /> GitHub
          </a>
          <a className="btn btn-primary" href="#download">
            <Icon name="download" size={18} /> Download
          </a>
        </div>
      </div>
    </header>
  );
}
