import { Fragment } from "react";
import { Icon } from "../components/icons";
import { securitySteps, securityPoints } from "../data/features";

export function Security() {
  return (
    <section className="section section-alt" id="security">
      <div className="container">
        <div className="section-head">
          <span className="eyebrow">Security by design</span>
          <h2 className="section-title">Encryption you can actually reason about.</h2>
          <p className="section-lead">
            A random Data Encryption Key — not your password — roots everything. That is what lets
            you reset a forgotten password without ever losing (or re-encrypting) your data.
          </p>
        </div>

        <div className="keypipe">
          {securitySteps.map((s, i) => (
            <Fragment key={s.label}>
              <div className="keypipe-step">
                <span className="keypipe-num">{i + 1}</span>
                <strong>{s.label}</strong>
                <span>{s.detail}</span>
              </div>
              {i < securitySteps.length - 1 && (
                <div className="keypipe-arrow" aria-hidden="true">
                  <Icon name="arrowRight" size={18} />
                </div>
              )}
            </Fragment>
          ))}
        </div>

        <div className="sec-points">
          {securityPoints.map((p) => (
            <div className="sec-point" key={p.title}>
              <Icon name="shield" size={18} className="sec-point-icon" />
              <strong>{p.title}</strong>
              <p>{p.body}</p>
            </div>
          ))}
        </div>

        <p className="sec-caveat">
          <Icon name="shield" size={14} /> Honest status: this is v{"0.1.0"} — Phase 0/1
          foundations. A formal external security review is a required gate before the encrypted
          sync release and hasn&apos;t happened yet.
        </p>
      </div>
    </section>
  );
}
