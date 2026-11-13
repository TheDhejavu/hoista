/**
 * Everything that happened to one project, newest first and grouped by day:
 * deploys at its provider, what agents and you changed, and what Hoista saw.
 * Routine events (Hoista refreshing what it knows) show only with All events.
 */
import { useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import type { ActivityItem } from '@shared/api';
import { useProjectActivityPages } from '@ui/api/projects';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { ProjectTab } from '../ProjectTab';
import { ActivityPanel } from './ActivityRecord';
import { ActivityRow } from './ProjectOverview';
import styles from './ProjectOverview.module.css';
import { DAY_MS } from '@shared/time';

type Who = 'all' | 'agent' | 'person' | 'system';

const WHO: Option<Who>[] = [
  { value: 'all', label: 'Anyone' },
  { value: 'agent', label: 'Agents' },
  { value: 'person', label: 'You' },
  { value: 'system', label: 'Providers and Hoista' },
];

const SHOW: Option<boolean>[] = [
  { value: false, label: 'Important' },
  { value: true, label: 'All events' },
];

function dayOf(at: string): string {
  const day = new Date(at);
  const today = new Date();
  const yesterday = new Date(Date.now() - DAY_MS);
  if (day.toDateString() === today.toDateString()) return 'Today';
  if (day.toDateString() === yesterday.toDateString()) return 'Yesterday';
  return day.toLocaleDateString('en-US', { weekday: 'long', month: 'short', day: 'numeric' });
}

export function ActivityPage() {
  const projectId = useRequiredParam('projectId');
  const pages = useProjectActivityPages(projectId);
  const [who, setWho] = useState<Who>('all');
  const [everything, setEverything] = useState(false);
  const [search, setSearch] = useSearchParams();

  const items = (pages.data?.pages ?? [])
    .flatMap((page) => page.items)
    .filter((item) => everything || !item.housekeeping)
    .filter((item) => who === 'all' || item.actor.kind === who);
  const asked = search.get('item');
  const selected = items.find((item) => item.id === asked) ?? items[0] ?? null;
  const days: { day: string; items: ActivityItem[] }[] = [];
  for (const item of items) {
    const day = dayOf(item.at);
    const last = days.at(-1);
    if (last && last.day === day) last.items.push(item);
    else days.push({ day, items: [item] });
  }

  return (
    <ProjectTab>
      <div className={styles.activityControls}>
        <h2 className={styles.activityTitle}>Activity</h2>
        <PillSelect label="By" value={who} options={WHO} onChange={setWho} />
        <PillSelect label="Show" value={everything} options={SHOW} onChange={setEverything} />
      </div>
      <QueryState query={pages} skeleton={<SkeletonRows count={6} columns={3} />}>
        {() => (
          <div className={selected ? styles.activityLayout : undefined}>
            <div>
              {days.length === 0 && <div className={styles.empty}>Nothing matches.</div>}
              {days.map((group) => (
                <section key={group.day} className={styles.activityDay}>
                  <h3>{group.day}</h3>
                  <ol className={styles.table}>
                    {group.items.map((item) => (
                      <ActivityRow
                        key={item.id}
                        projectId={projectId}
                        item={item}
                        selected={item.id === selected?.id}
                        onSelect={() => {
                          setSearch({ item: item.id }, { replace: true });
                        }}
                      />
                    ))}
                  </ol>
                </section>
              ))}
              {pages.hasNextPage && (
                <Button
                  busy={pages.isFetchingNextPage}
                  busyLabel="Loading"
                  onClick={() => {
                    void pages.fetchNextPage();
                  }}
                >
                  Show older
                </Button>
              )}
            </div>
            {selected && <ActivityPanel key={selected.id} projectId={projectId} item={selected} />}
          </div>
        )}
      </QueryState>
    </ProjectTab>
  );
}
