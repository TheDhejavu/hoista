/**
 * The field a provider token is pasted into, everywhere Hoista asks for one. It
 * says plainly that a 1Password secret reference works too, and shows which
 * of the two it was given, so keeping tokens in 1Password is never hidden.
 */
import { KeyRound } from 'lucide-react';
import { SecretField } from './Field';
import { ExternalLink } from './ExternalLink';
import styles from './TokenField.module.css';

/** 1Password's own page on secret references (Copy Secret Reference). */
const REFERENCES_HELP = 'https://developer.1password.com/docs/cli/secret-references/';

export function TokenField({
  label,
  value,
  placeholder,
  onChange,
}: {
  label: string;
  value: string;
  placeholder?: string;
  onChange: (value: string) => void;
}) {
  const reference = value.trim().startsWith('op://');

  return (
    <div className={styles.wrap}>
      <SecretField
        label={label}
        value={value}
        placeholder={placeholder}
        onChange={onChange}
        hint={
          reference
            ? 'A 1Password reference. The token stays in 1Password; Hoista keeps only the reference and checks it works before saving.'
            : 'Saved to this computer’s keychain. Hoista checks it works before saving.'
        }
      />
      <div className={styles.onePassword}>
        <KeyRound size={15} aria-hidden="true" />
        <span>
          <b>Keep it in 1Password instead.</b> Paste its secret reference, such as{' '}
          <code>op://Private/Hosting/token</code>, in place of the token. Hoista reads it from
          1Password each time it needs it.{' '}
          <ExternalLink url={REFERENCES_HELP} variant="text">
            How to copy one
          </ExternalLink>
        </span>
      </div>
    </div>
  );
}
