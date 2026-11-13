/**
 * A modal dialog built on the native `<dialog>` element, which handles focus,
 * Escape to close and the backdrop for us.
 *
 * Render it only while it should be open; it opens itself on mount. `large`
 * is for dialogs that hold a whole area, such as Settings: the body fills the
 * dialog and scrolls on its own.
 */
import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  type ReactNode,
} from 'react';
import { cx } from '@ui/lib/cx';
import styles from './Dialog.module.css';

/**
 * Inside a `shell` dialog: a step of the same window. A dialog rendered
 * here draws its header, body and footer in that window instead of opening
 * one of its own, so moving between steps never stacks windows.
 */
const InShell = createContext(false);

export function Dialog({
  title,
  eyebrow,
  icon,
  onClose,
  onBack,
  backLabel,
  actions,
  sideAction,
  large = false,
  wide = false,
  bare = false,
  plain = false,
  shell = false,
  depth,
  children,
}: {
  title: string;
  /** Small grey line under the title, for context or status, such as `moodboard · Environment`. */
  eyebrow?: ReactNode;
  /** A logo or icon beside the title. */
  icon?: ReactNode;
  onClose: () => void;
  /**
   * A step reached from another one in the same flow: a back link above the
   * title, named after where it goes (`backLabel`), like a sheet's back
   * button in macOS. Close still leaves the whole flow.
   */
  onBack?: () => void;
  backLabel?: string;
  /** Buttons along the bottom right, Cancel first and the main action last. */
  actions?: ReactNode;
  /** A quieter action on the bottom left, such as Disconnect. */
  sideAction?: ReactNode;
  large?: boolean;
  /** Wider than usual, for forms laid out in two columns. */
  wide?: boolean;
  /** No header, padding or footer: the children draw the whole dialog, such as Settings. */
  bare?: boolean;
  /**
   * No header, padding or footer at a dialog's usual size: the children draw
   * it all, for a dialog laid out differently, such as a drop zone or a result
   * with a picture at the top. They provide their own close button.
   */
  plain?: boolean;
  /**
   * The window only: a dialog among the children draws itself inside it, as
   * the next step of the same flow. Swapping between a shell and a normal
   * dialog at the same place keeps the one window open.
   */
  shell?: boolean;
  /**
   * How deep this step is in a multi-step flow (0 for the first). When it
   * changes, the step slides in like a navigation push: from the right going
   * deeper, from the left coming back. The window itself stays put.
   */
  depth?: number;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  // A press that began outside the dialog's box, on the dimmed page.
  const pressedOutside = useRef(false);
  const outside = (event: { clientX: number; clientY: number }) => {
    const box = ref.current?.getBoundingClientRect();
    return (
      !!box &&
      (event.clientX < box.left ||
        event.clientX > box.right ||
        event.clientY < box.top ||
        event.clientY > box.bottom)
    );
  };

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    dialog.showModal();
    // Focus the dialog itself rather than its first button, so the close
    // button doesn't light up with a focus ring as the dialog opens.
    dialog.focus();
  }, []);

  const inShell = useContext(InShell);
  const contentRef = useRef<HTMLDivElement>(null);
  // Compared with the step shown before in the same window, kept on the
  // window itself, since a step may be drawn by a different dialog.
  useLayoutEffect(() => {
    const step = contentRef.current;
    const frame = step?.closest('dialog');
    if (!step || !frame || depth === undefined) return;
    const before = frame.dataset.depth;
    frame.dataset.depth = String(depth);
    if (before === undefined || Number(before) === depth) return;
    const right = styles.fromRight ?? '';
    const left = styles.fromLeft ?? '';
    step.classList.remove(right, left);
    // Restart the animation when steps change quickly.
    step.getBoundingClientRect();
    step.classList.add(depth > Number(before) ? right : left);
  }, [depth]);
  const content = (
    <div ref={contentRef} className={styles.content}>
      {onBack && (
        <button type="button" className={styles.back} onClick={onBack}>
          <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M15 6l-6 6 6 6" />
          </svg>
          {backLabel ?? 'Back'}
        </button>
      )}
      <div className={styles.header}>
        <div className={styles.heading}>
          {icon && <span className={styles.icon}>{icon}</span>}
          <div className={styles.headingText}>
            <h2 id="dialog-title" className={styles.title}>
              {title}
            </h2>
            {eyebrow && <span className={styles.eyebrow}>{eyebrow}</span>}
          </div>
        </div>
        <button type="button" className={styles.close} aria-label="Close" onClick={onClose}>
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            aria-hidden="true"
          >
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      </div>
      {/* Dialogs opened from this step are windows of their own again. */}
      <InShell.Provider value={false}>
        {large ? <div className={styles.body}>{children}</div> : children}
      </InShell.Provider>
      {(actions || sideAction) && (
        <div className={styles.footer}>
          {sideAction}
          {actions && <div className={styles.actions}>{actions}</div>}
        </div>
      )}
    </div>
  );

  // A step of a shell dialog: drawn in that window.
  if (inShell) return content;

  return (
    <dialog
      ref={ref}
      tabIndex={-1}
      className={cx(
        styles.dialog,
        large && styles.large,
        wide && styles.wide,
        bare && styles.bare,
        plain && styles.plain,
      )}
      aria-labelledby="dialog-title"
      // A dialog opened over this one sits inside it; its close mustn't close this one too.
      onClose={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      // A click on the dimmed page around it closes it, like Escape. One
      // that starts inside (selecting text) and ends outside doesn't.
      onMouseDown={(event) => {
        pressedOutside.current = event.target === event.currentTarget && outside(event);
      }}
      onClick={(event) => {
        if (pressedOutside.current && event.target === event.currentTarget && outside(event)) {
          onClose();
        }
        pressedOutside.current = false;
      }}
    >
      {bare || plain ? (
        children
      ) : shell ? (
        <InShell.Provider value={true}>{children}</InShell.Provider>
      ) : (
        content
      )}
    </dialog>
  );
}

/** Body text in a confirmation dialog: paragraphs and a short list of consequences. */
export function DialogText({ children }: { children: ReactNode }) {
  return <div className={styles.text}>{children}</div>;
}
