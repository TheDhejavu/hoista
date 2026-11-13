/**
 * Labelled text inputs. Every input has a visible label above it.
 * Dropdowns are in `./Select`.
 */
import { useId, useState, type ReactNode } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { cx } from '@ui/lib/cx';
import styles from './Field.module.css';

interface InputProps {
  label: string;
  value: string;
  placeholder?: string;
  onChange: (value: string) => void;
  /** One grey line under the input. */
  hint?: string;
  /** Take up the free width in a row of fields. */
  grow?: boolean;
}

export function TextField(props: InputProps) {
  return <LabelledInput type="text" {...props} />;
}

/**
 * A password-style input for tokens and secret values.
 *
 * The value lives only in the parent's state until it is sent, and the parent
 * clears it after. Autocomplete and spellcheck are off so the system never
 * remembers or underlines what was pasted.
 */
export function SecretField({
  revealable = false,
  ...props
}: InputProps & {
  /** An eye button in the field that shows what was typed, for passphrases you type yourself. */
  revealable?: boolean;
}) {
  const [shown, setShown] = useState(false);
  return (
    <LabelledInput
      type={shown ? 'text' : 'password'}
      autoComplete="off"
      spellCheck={false}
      {...props}
      reveal={
        revealable ? (
          <button
            type="button"
            className={styles.reveal}
            aria-label={
              shown ? `Hide ${props.label.toLowerCase()}` : `Show ${props.label.toLowerCase()}`
            }
            aria-pressed={shown}
            onClick={() => {
              setShown(!shown);
            }}
          >
            {shown ? <EyeOff size={16} aria-hidden="true" /> : <Eye size={16} aria-hidden="true" />}
          </button>
        ) : undefined
      }
    />
  );
}

function LabelledInput({
  label,
  value,
  placeholder,
  onChange,
  hint,
  grow = false,
  type,
  autoComplete,
  spellCheck,
  reveal,
}: InputProps & {
  type: 'text' | 'password';
  autoComplete?: string;
  spellCheck?: boolean;
  /** A button drawn inside the input's right end. */
  reveal?: ReactNode;
}) {
  const id = useId();
  const input = (
    <input
      id={id}
      type={type}
      className={cx(styles.control, reveal !== undefined && styles.withReveal)}
      value={value}
      placeholder={placeholder}
      autoComplete={autoComplete}
      spellCheck={spellCheck}
      onChange={(event) => {
        onChange(event.target.value);
      }}
    />
  );

  return (
    <div className={cx(styles.field, grow && styles.grow)}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      {reveal ? (
        <span className={styles.revealWrap}>
          {input}
          {reveal}
        </span>
      ) : (
        input
      )}
      {hint && <span className={styles.hint}>{hint}</span>}
    </div>
  );
}
