/**
 * Get help, as a dialog: the docs pages for when something isn't working,
 * reporting a bug, and a diagnostic report to attach to it.
 */
import { useSaveDiagnosticReport } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ExternalLink } from '@ui/components/ExternalLink';
import { Dialog } from '@ui/components/Dialog';
import { Panel } from '@ui/components/Panel';
import styles from './HelpDialog.module.css';

const LINKS = [
  { label: 'Documentation', url: 'https://docs.hoista.dev' },
  {
    label: 'When something breaks',
    url: 'https://docs.hoista.dev/getting-started/support#when-something-breaks',
  },
  { label: 'Fix a failed deploy', url: 'https://docs.hoista.dev/examples/failed-deployment' },
  { label: 'Report a bug on GitHub', url: 'https://github.com/TheDhejavu/hoista/issues/new' },
];

export function HelpDialog({ onClose }: { onClose: () => void }) {
  const report = useSaveDiagnosticReport();

  return (
    <Dialog title="Get help" onClose={onClose}>
      <p className={styles.intro}>
        Hoista is free and open source. Where to look when something isn't working.
      </p>
      <div className={styles.columns}>
        <Panel padded>
          <section className={styles.section}>
            <h3 className={styles.title}>Docs and bug reports</h3>
            <div className={styles.links}>
              {LINKS.map((link) => (
                <ExternalLink key={link.url} url={link.url} variant="text" className={styles.link}>
                  {link.label}
                </ExternalLink>
              ))}
            </div>
            <p className={styles.muted}>
              Or ask your agent: "why is my deploy failing?" It can read your logs.
            </p>
          </section>
        </Panel>

        <Panel padded>
          <section className={styles.section}>
            <h3 className={styles.title}>Diagnostic report</h3>
            <p className={styles.muted}>
              Your project list, recent deploy logs and the Hoista version. Never includes keys,
              tokens or secret values.
            </p>
            <div>
              <Button
                variant="primary"
                disabled={report.isPending}
                onClick={() => {
                  report.mutate();
                }}
              >
                Save report
              </Button>
            </div>
            {report.data && <p className={styles.saved}>Saved to {report.data.path}</p>}
            <ErrorMessage error={report.error} />
          </section>
        </Panel>
      </div>
    </Dialog>
  );
}
