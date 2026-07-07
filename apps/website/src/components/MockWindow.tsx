import { Icon } from "./icons";

/**
 * A stylized "screenshot" of the app, drawn in HTML/CSS (no image asset). Shows
 * a sidebar, a page with a few blocks, the slash-command menu, and the floating
 * "Ask AI" button — enough to read as the product at a glance.
 */
export function MockWindow() {
  return (
    <div className="mock" role="img" aria-label="A preview of the encrypted notes app">
      <div className="mock-bar">
        <span className="dot" />
        <span className="dot" />
        <span className="dot" />
        <div className="mock-lock">
          <Icon name="lock" size={13} /> vault unlocked
        </div>
      </div>
      <div className="mock-body">
        <aside className="mock-side">
          <div className="mock-side-item active">Getting started</div>
          <div className="mock-side-item">Q3 planning</div>
          <div className="mock-side-item">Invoices</div>
          <div className="mock-side-item">Reading list</div>
          <div className="mock-side-item muted">+ New page</div>
        </aside>
        <div className="mock-page">
          <div className="mock-h1">Getting started</div>
          <div className="mock-line" />
          <div className="mock-line short" />
          <div className="mock-todo">
            <span className="mock-check" /> Encrypt everything at rest
          </div>
          <div className="mock-todo">
            <span className="mock-check done" /> Work fully offline
          </div>
          <div className="mock-slash">
            <span className="mock-slash-caret">/</span>
            <ul>
              <li className="ai">Ask AI ✨</li>
              <li>Heading 1</li>
              <li>To-do list</li>
              <li>Code</li>
            </ul>
          </div>
        </div>
      </div>
      <button className="mock-fab" aria-hidden="true">
        <Icon name="sparkle" size={20} />
      </button>
    </div>
  );
}
