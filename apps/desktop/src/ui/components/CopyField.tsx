/**
 * Text to copy, such as a link or a command, with a Copy button. One line by
 * default; `lines` shows a short file, such as JSON, as it is.
 */
import { cx } from '@ui/lib/cx';
import { useCopy } from '@ui/lib/useCopy';
import { Button } from './Button';
import styles from './CopyField.module.css';

export function CopyField({
  text,
  label,
  lines = false,
}: {
  text: string;
  label: string;
  lines?: boolean;
}) {
  const { copied, copy } = useCopy();

  return (
    <div className={cx(styles.field, lines && styles.lines)}>
      {lines ? (
        <pre className={cx(styles.code, styles.block)}>{text}</pre>
      ) : (
        <code className={styles.code} title={text}>
          {text}
        </code>
      )}
      <Button
        aria-label={`Copy ${label}`}
        onClick={() => {
          copy(text);
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </Button>
    </div>
  );
}
