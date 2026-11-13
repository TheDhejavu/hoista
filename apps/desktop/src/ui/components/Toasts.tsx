/**
 * The notes from lib/toast, stacked at the bottom of the window: an icon for
 * their kind, the text, and an action when they have one. Hovering keeps
 * them up. Screen readers hear them as they appear; problems are announced
 * straight away. The stack lives in the top layer and is raised again with
 * each note, so it shows above an open dialog (dialogs are in the top layer too).
 */
import { AlertCircle, Check, Info, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import {
  dismissToast,
  pauseToasts,
  resumeToasts,
  subscribeToToasts,
  type Toast,
} from '@ui/lib/toast';
import { cx } from '@ui/lib/cx';
import styles from './Toasts.module.css';

const ICONS = { success: Check, error: AlertCircle, info: Info } as const;

export function Toasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const stack = useRef<HTMLDivElement>(null);
  useEffect(() => subscribeToToasts(setToasts), []);

  useEffect(() => {
    const element = stack.current;
    if (!element || typeof element.showPopover !== 'function') return;
    if (element.matches(':popover-open')) element.hidePopover();
    if (toasts.length > 0) element.showPopover();
    else resumeToasts();
  }, [toasts]);

  return (
    <div
      ref={stack}
      popover="manual"
      className={styles.stack}
      onMouseEnter={pauseToasts}
      onMouseLeave={resumeToasts}
    >
      {toasts.map((toast) => {
        const Icon = ICONS[toast.tone];
        if (toast.detail) return <ToastCard key={toast.id} toast={toast} />;
        return (
          <div
            key={toast.id}
            className={cx(styles.toast, styles[toast.tone], toast.leaving && styles.leaving)}
            role={toast.tone === 'error' ? 'alert' : 'status'}
            aria-live={toast.tone === 'error' ? 'assertive' : 'polite'}
          >
            <Icon size={14} strokeWidth={2.6} aria-hidden="true" className={styles.icon} />
            <span className={styles.text}>{toast.text}</span>
            {toast.action && (
              <button
                type="button"
                className={styles.action}
                onClick={() => {
                  toast.action?.onClick();
                  dismissToast(toast.id);
                }}
              >
                {toast.action.label}
              </button>
            )}
            {(toast.action ?? toast.tone === 'error') && (
              <button
                type="button"
                className={styles.close}
                aria-label="Dismiss"
                onClick={() => {
                  dismissToast(toast.id);
                }}
              >
                <X size={13} aria-hidden="true" />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

/** A note with a detail line: a card in the corner with its kind's badge, the text and a close button. */
function ToastCard({ toast }: { toast: Toast }) {
  const Icon = ICONS[toast.tone];
  return (
    <div
      className={cx(styles.card, styles[toast.tone], toast.leaving && styles.leaving)}
      role={toast.tone === 'error' ? 'alert' : 'status'}
      aria-live={toast.tone === 'error' ? 'assertive' : 'polite'}
    >
      <span className={styles.badge}>
        <Icon size={12} strokeWidth={3.5} aria-hidden="true" />
      </span>
      <span className={styles.cardText}>
        <b>{toast.text}</b>
        <span>{toast.detail}</span>
      </span>
      <button
        type="button"
        className={styles.cardClose}
        aria-label="Dismiss"
        onClick={() => {
          dismissToast(toast.id);
        }}
      >
        <X size={14} strokeWidth={2.2} aria-hidden="true" />
      </button>
    </div>
  );
}
