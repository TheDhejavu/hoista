/**
 * First-run setup: a rail on the left with the three steps (Providers,
 * Projects, Agents) and where you are, and the current step beside it. The
 * rail also offers bringing Hoista over from another computer, and skipping
 * setup. Finishing or skipping saves the profile, as the welcome screen did.
 */
import { useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router-dom';
import { Check } from 'lucide-react';
import { useStartHoista } from '@ui/api/accounts';
import { Mascot } from '@ui/components/Mascot';
import { cx } from '@ui/lib/cx';
import { listOf, plural } from '@ui/lib/format';
import { showToast } from '@ui/lib/toast';
import { ImportTransferDialog } from '@ui/screens/settings/transfer/ImportTransferDialog';
import { serviceName } from '@shared/providers';
import { usePicked, type SetupContext, type SetupStepId } from './useSetup';
import styles from './Setup.module.css';

/** Which rail step each screen belongs to. */
const RAIL_INDEX: Record<SetupStepId, number> = { providers: 0, connect: 0, projects: 1, ai: 2 };

export function SetupLayout() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const picked = usePicked();
  const start = useStartHoista();
  const [imported, setImported] = useState<number | null>(null);
  const [moving, setMoving] = useState(false);
  const segment = pathname.split('/')[2] ?? '';
  const current = segment in RAIL_INDEX ? RAIL_INDEX[segment as SetupStepId] : 0;
  const names = picked.map((id) => serviceName(id));

  const rail = [
    {
      title: 'Providers',
      text: current > 0 && names.length > 0 ? listOf(names) : 'Where your projects run',
    },
    {
      title: 'Projects',
      text:
        current > 1 && imported !== null
          ? `${plural(imported, 'project')} added`
          : 'Pick what Hoista watches',
    },
    { title: 'Agents', text: 'Let your agent work on them' },
  ];

  function open(note?: { title: string; detail: string }): void {
    start.mutate(undefined, {
      onSuccess: () => {
        void navigate('/');
        if (note) showToast(note.title, { detail: note.detail });
      },
    });
  }

  const context: SetupContext = {
    imported,
    setImported,
    finish: (agent) => {
      const what =
        imported && imported > 0
          ? `${plural(imported, 'project')} from ${listOf(names)}.`
          : names.length > 0
            ? `${listOf(names)} ${names.length === 1 ? 'is' : 'are'} connected.`
            : 'Connect a provider any time from Accounts.';
      open({
        title: 'Hoista is set up',
        detail: agent ? `${what} Ask ${agent} what’s running.` : what,
      });
    },
    skip: () => {
      open();
    },
    finishing: start.isPending,
    finishError: start.error,
  };

  return (
    <div className={styles.setup}>
      <aside className={styles.rail} aria-label="Setup steps">
        <div className={styles.railHead}>
          <span className={styles.railMark}>
            <Mascot size={18} />
          </span>
          <b>Set up Hoista</b>
        </div>
        <ol className={styles.railSteps}>
          {rail.map((item, index) => {
            const state = index < current ? 'done' : index === current ? 'now' : 'later';
            return (
              <li
                key={item.title}
                className={styles.railStep}
                aria-current={state === 'now' ? 'step' : undefined}
              >
                <span className={cx(styles.railDot, styles[state])}>
                  {state === 'done' ? (
                    <Check size={13} strokeWidth={3.5} aria-label="Done" />
                  ) : (
                    index + 1
                  )}
                </span>
                <span className={styles.railText}>
                  <span className={cx(styles.railTitle, state === 'later' && styles.railLater)}>
                    {item.title}
                  </span>
                  <span className={styles.railSub}>{item.text}</span>
                </span>
              </li>
            );
          })}
        </ol>
        <div className={styles.railLinks}>
          <button
            type="button"
            className={styles.railMoving}
            onClick={() => {
              setMoving(true);
            }}
          >
            Moving from another computer?
          </button>
          <button
            type="button"
            className={styles.railSkip}
            disabled={start.isPending}
            onClick={context.skip}
          >
            {start.isPending ? 'Opening Hoista' : 'Skip setup'}
          </button>
        </div>
      </aside>
      <Outlet context={context} />
      {moving && (
        <ImportTransferDialog
          onClose={() => {
            setMoving(false);
          }}
          onImported={() => {
            void navigate('/');
          }}
        />
      )}
    </div>
  );
}
