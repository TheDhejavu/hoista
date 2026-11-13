/**
 * What needs you, as a row of cards across the top of Projects: approvals first,
 * then changes Hoista didn't hear back about, failed deploys,
 * savings and accounts. Each card has the one
 * action that deals with it. Shows the three most urgent, as in the design;
 * hidden when nothing needs you.
 */
import { useState } from 'react';
import {
  CircleHelp,
  KeyRound,
  PiggyBank,
  Plug,
  ShieldQuestion,
  type LucideIcon,
} from 'lucide-react';
import type { AttentionItem } from '@shared/api';
import { useAttention } from '@ui/api/projects';
import { useInEnvironment } from '@ui/lib/useInEnvironment';
import { AgentIcon } from '@ui/components/AgentIcon';
import { CopyPromptButton } from '@ui/components/AgentPrompt';
import { Button, ButtonLink } from '@ui/components/Button';
import { cx } from '@ui/lib/cx';
import { reconnectPath } from '@shared/reconnect';
import styles from './NeedsStrip.module.css';
import { UnclearDialog } from './UnclearDialog';
import { useProjectNames } from '@ui/lib/useProjectNames';

const ORDER: AttentionItem['kind'][] = ['approval', 'unclear', 'failure', 'saving', 'account'];
const ICONS: Record<AttentionItem['kind'], LucideIcon> = {
  approval: ShieldQuestion,
  failure: KeyRound,
  saving: PiggyBank,
  account: Plug,
  unclear: CircleHelp,
};
/** An agent waiting on you to reconnect a provider comes first. */
function urgency(item: AttentionItem): number {
  return item.waiting_on ? 0.5 : ORDER.indexOf(item.kind);
}

/** Cards shown, most urgent first. */
const SHOWN = 3;

export function NeedsStrip() {
  const attention = useAttention();
  const inEnvironment = useInEnvironment();
  const items = (attention.data?.items ?? [])
    .filter((item) => item.kind !== 'approval' && inEnvironment(item.project_id))
    .sort((a, b) => urgency(a) - urgency(b));
  if (items.length === 0) return null;

  const shown = items.slice(0, SHOWN);

  return (
    <section aria-label="Needs you" className={styles.strip}>
      {shown.map((item) => (
        <NeedCard key={item.id} item={item} />
      ))}
    </section>
  );
}

function NeedCard({ item }: { item: AttentionItem }) {
  const Icon = ICONS[item.kind];

  return (
    <div className={styles.card}>
      {item.agent ? (
        <AgentIcon name={item.agent} />
      ) : (
        <span className={cx(styles.icon, styles[item.tone])}>
          <Icon size={14} aria-hidden="true" />
        </span>
      )}
      <span className={styles.text}>
        <span className={styles.title}>{item.title}</span>
        <span className={styles.body}>{item.body}</span>
      </span>
      <NeedAction item={item} />
    </div>
  );
}

function NeedAction({ item }: { item: AttentionItem }) {
  const projectName = useProjectNames();
  const name = projectName(item.project_id);
  switch (item.kind) {
    case 'approval':
      // Approvals are answered in the agent that asked.
      return null;
    case 'failure':
      return item.project_id ? (
        <ButtonLink to={`/projects/${item.project_id}`}>Open {name}</ButtonLink>
      ) : null;
    case 'saving':
      return item.project_id ? (
        <CopyPromptButton
          icon={false}
          prompt={`In Hoista, ${name} has resources nobody has used lately. List them with their usage so far, and ask me before pausing or removing any.`}
        />
      ) : null;
    case 'unclear':
      return item.operation ? <AnswerUnclear operation={item.operation} /> : null;
    case 'account':
      return item.waiting_on ? (
        <ButtonLink to={reconnectPath(item.waiting_on)} variant="accent">
          Reconnect
        </ButtonLink>
      ) : (
        <ButtonLink to="/accounts">Open accounts</ButtonLink>
      );
  }
}

function AnswerUnclear({ operation }: { operation: NonNullable<AttentionItem['operation']> }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        variant="accent"
        onClick={() => {
          setOpen(true);
        }}
      >
        Answer
      </Button>
      {open && (
        <UnclearDialog
          operation={operation}
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </>
  );
}
