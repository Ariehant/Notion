import { useEffect, useState } from "react";
import { createVault, recoverVault, unlockVault, vaultExists } from "../bridge";

type Mode = "loading" | "create" | "unlock" | "recover" | "show-recovery";

interface VaultGateProps {
  /** Called once the vault is open so the app can load the workspace. */
  onUnlocked: () => void;
}

/**
 * The lock screen. Decides create-vs-unlock from whether a vault exists on disk,
 * runs the (Argon2id) create/unlock/recover flows, and — on creation — shows the
 * one-time recovery code before entering the app (audit §2.5: no silent loss).
 */
export function VaultGate({ onUnlocked }: VaultGateProps) {
  const [mode, setMode] = useState<Mode>("loading");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [recoveryCode, setRecoveryCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newRecovery, setNewRecovery] = useState<string | null>(null);

  useEffect(() => {
    void vaultExists()
      .then((exists) => setMode(exists ? "unlock" : "create"))
      .catch((err) => {
        console.error(err);
        setMode("create");
      });
  }, []);

  const reset = () => {
    setPassword("");
    setConfirm("");
    setRecoveryCode("");
    setError(null);
  };

  const handleCreate = async () => {
    if (password.length < 8) return setError("Use at least 8 characters.");
    if (password !== confirm) return setError("Passwords do not match.");
    setBusy(true);
    setError(null);
    try {
      const code = await createVault(password);
      setNewRecovery(code);
      setMode("show-recovery");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleUnlock = async () => {
    setBusy(true);
    setError(null);
    try {
      await unlockVault(password);
      onUnlocked();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleRecover = async () => {
    if (password.length < 8) return setError("Use at least 8 characters.");
    if (password !== confirm) return setError("Passwords do not match.");
    setBusy(true);
    setError(null);
    try {
      await recoverVault(recoveryCode.trim(), password);
      onUnlocked();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const onEnter = (fn: () => void) => (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !busy) fn();
  };

  return (
    <div className="vault-gate">
      <div className="vault-card">
        <h1>Notion</h1>
        <p className="tagline">Offline-first · end-to-end encrypted</p>

        {mode === "loading" && <p>Loading…</p>}

        {mode === "create" && (
          <>
            <p className="hint">
              Create a vault. Your password derives the key that encrypts everything on this device.
              There is no server that can reset it — keep the recovery code you'll receive next.
            </p>
            <input
              type="password"
              placeholder="New password"
              autoFocus
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
            <input
              type="password"
              placeholder="Confirm password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              onKeyDown={onEnter(handleCreate)}
            />
            <button type="button" disabled={busy} onClick={handleCreate}>
              {busy ? "Creating…" : "Create vault"}
            </button>
          </>
        )}

        {mode === "unlock" && (
          <>
            <input
              type="password"
              placeholder="Password"
              autoFocus
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={onEnter(handleUnlock)}
            />
            <button type="button" disabled={busy} onClick={handleUnlock}>
              {busy ? "Unlocking…" : "Unlock"}
            </button>
            <button
              type="button"
              className="link"
              onClick={() => {
                reset();
                setMode("recover");
              }}
            >
              Forgot password? Use recovery code
            </button>
          </>
        )}

        {mode === "recover" && (
          <>
            <p className="hint">
              Enter your recovery code and choose a new password. Your data is preserved.
            </p>
            <textarea
              className="recovery-input"
              placeholder="Recovery code"
              autoFocus
              value={recoveryCode}
              onChange={(e) => setRecoveryCode(e.target.value)}
            />
            <input
              type="password"
              placeholder="New password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
            <input
              type="password"
              placeholder="Confirm new password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              onKeyDown={onEnter(handleRecover)}
            />
            <button type="button" disabled={busy} onClick={handleRecover}>
              {busy ? "Recovering…" : "Recover & set password"}
            </button>
            <button
              type="button"
              className="link"
              onClick={() => {
                reset();
                setMode("unlock");
              }}
            >
              Back to unlock
            </button>
          </>
        )}

        {mode === "show-recovery" && newRecovery && (
          <>
            <p className="hint">
              <strong>Save this recovery code now.</strong> It is shown only once and is the only
              way to recover your data if you forget your password.
            </p>
            <pre className="recovery-code">{newRecovery}</pre>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard?.writeText(newRecovery).catch(() => {});
              }}
            >
              Copy code
            </button>
            <button type="button" className="primary" onClick={onUnlocked}>
              I've saved it — open Notion
            </button>
          </>
        )}

        {error && <p className="error">{error}</p>}
      </div>
    </div>
  );
}
