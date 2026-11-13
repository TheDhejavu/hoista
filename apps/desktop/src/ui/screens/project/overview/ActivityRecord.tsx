/**
 * The record behind one activity line, beside the list like an audit entry:
 * who did it and when, what Hoista recorded, and for an agent's change the
 * exact call it sent (secret values are never kept) and how it ended.
 */
import { Link } from 'react-router-dom';
import type { ActivityItem } from '@shared/api';
import { useActivityRecord } from '@ui/api/projects';
import { ActorIcon, AgentIcon } from '@ui/components/AgentIcon';
import { SkeletonLines } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import styles from './ActivityRecord.module.css';

const LINKS = { logs: 'See logs', deploys: 'See deploys', map: 'See the map' } as const;

function when(at: string): string {
  const day = new Date(at);
  const time = day.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  if (day.toDateString() === new Date().toDateString()) return `Today ${time}`;
  return `${day.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })} ${time}`;
}

function statusTone(status: string, tone: ActivityItem['tone']): ActivityItem['tone'] {
  if (status === 'Refused' || status === 'You declined it') return 'bad';
  if (status === 'Waiting for your approval') return 'warn';
  return status === 'Completed' ? 'good' : tone;
}

export function ActivityPanel({ projectId, item }: { projectId: string; item: ActivityItem }) {
  const record = useActivityRecord(projectId, item.id);
  const entry = record.data;
  return (
    <aside className={styles.panel} aria-label="What happened">
      <div className={styles.top}>
        <div className={styles.who}>
          {item.actor.kind === 'agent' ? (
            <AgentIcon name={item.actor.name} small />
          ) : (
            <ActorIcon actor={item.actor} />
          )}
          <b>{item.actor.name}</b>
          <span className={styles.when}>{when(item.at)}</span>
        </div>
        <div className={styles.title}>{item.title}</div>
        {entry?.tool && <div className={styles.kind}>{entry.tool}</div>}
      </div>
      <div className={styles.body}>
        {record.isPending && <SkeletonLines count={4} />}
        {entry?.sent && (
          <>
            <h3>What it sent</h3>
            <pre className={styles.code}>{entry.sent}</pre>
          </>
        )}
        {entry?.status && (
          <>
            <h3>Status</h3>
            <p className={styles.line}>
              <span className={cx(styles.dot, styles[statusTone(entry.status, item.tone)])} />
              <span>{entry.status}</span>
            </p>
          </>
        )}
        {entry && entry.happened.length > 0 && (
          <>
            <h3>What happened</h3>
            {entry.happened.map((line) => (
              <p key={line} className={styles.line}>
                <span className={cx(styles.dot, styles[item.tone])} />
                <span>{line}</span>
              </p>
            ))}
          </>
        )}
        {item.link && (
          <Link to={`/projects/${projectId}/${item.link}`} className={styles.review}>
            {LINKS[item.link]} →
          </Link>
        )}
      </div>
    </aside>
  );
}
