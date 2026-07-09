import { Icon } from "./icons";

/**
 * A pure CSS/JSX phone illustration for the Android hero — a home-screen agenda
 * widget above an open, encrypted note, plus a biometric-unlock chip. No image
 * assets, so it themes with the page and stays crisp at any size.
 */
export function PhoneMock() {
  return (
    <div className="phone" aria-hidden="true">
      <div className="phone-notch" />
      <div className="phone-screen">
        <div className="phone-widget">
          <span className="phone-widget-head">
            <Icon name="calendar" size={13} /> Today
          </span>
          <span className="phone-widget-row">
            <b>09:30</b> Standup
          </span>
          <span className="phone-widget-row muted">
            <b>15:00</b> Q3 planning
          </span>
        </div>

        <div className="phone-note">
          <span className="phone-note-title">Weekly notes</span>
          <span className="phone-line" />
          <span className="phone-line short" />
          <span className="phone-todo">
            <i className="phone-check" /> Ship the Android build
          </span>
          <span className="phone-line" />
        </div>

        <div className="phone-unlock">
          <Icon name="fingerprint" size={16} /> Unlock
        </div>
      </div>
    </div>
  );
}
