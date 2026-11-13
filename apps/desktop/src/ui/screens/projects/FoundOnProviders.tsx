/**
 * What connecting your providers found and isn't in Hoista yet, summed up per
 * connected account (a Railway workspace): how many projects and resources.
 * "Review and add" opens the full list, by workspace and project, to pick
 * from. Suggestions to merge projects from one provider project show here too.
 */
import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { ProviderId } from '@shared/api';
import { projectKeys } from '@ui/api/keys';
import { useCombineProjects, useFoundOnProviders } from '@ui/api/imports';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { useShownRemote } from '@ui/api/settings';
import { serviceName } from '@shared/providers';
import { FoundDialog, sections } from './FoundDialog';
import styles from './FoundOnProviders.module.css';

export function FoundOnProviders({ onConnect }: { onConnect?: (provider: ProviderId) => void }) {
  const found = useFoundOnProviders();
  const combine = useCombineProjects();
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);

  const data = found.data;
  // A remote that isn't answering is said once, in the strip above; this
  // card would only repeat it.
  const shown = useShownRemote();
  if (shown?.down) return null;
  if (found.isError) {
    return (
      <section className={styles.section} aria-labelledby="found-title">
        <header className={styles.header}>
          <h2 id="found-title" className={styles.title}>
            Found on your providers
          </h2>
        </header>
        <div className={styles.rows}>
          <div className={styles.row} role="alert">
            <span className={styles.text}>
              <span>Couldn’t read your providers</span>
              <span className={styles.meta}>{found.error.message}</span>
            </span>
            <span className={styles.actions}>
              <Button
                busy={found.isFetching}
                busyLabel="Checking"
                onClick={() => {
                  void queryClient.invalidateQueries({ queryKey: projectKeys.all });
                }}
              >
                Check again
              </Button>
            </span>
          </div>
        </div>
      </section>
    );
  }
  if (!data || (data.resources.length === 0 && data.combine.length === 0)) return null;
  const list = sections(data);

  return (
    <section className={styles.section} aria-labelledby="found-title">
      <header className={styles.header}>
        <h2 id="found-title" className={styles.title}>
          Found on your providers
        </h2>
        <p className={styles.note}>
          Already running there. Add them to see status, logs and deploys here and let your agent
          work on them.
        </p>
      </header>

      <ul className={styles.rows}>
        {data.combine.map((item) => (
          <li key={item.from.id} className={styles.row}>
            <ServiceIcon id={item.provider} size={18} />
            <span className={styles.text}>
              <span>
                {item.from.name} belongs with {item.into.name}
              </span>
              <span className={styles.meta}>
                {item.reason} · combine them into one project with both resources
              </span>
            </span>
            <span className={styles.actions}>
              <Button
                variant="primary"
                disabled={combine.isPending}
                onClick={() => {
                  combine.mutate({ from: item.from.id, into: item.into.id });
                }}
              >
                {combine.isPending ? 'Combining' : `Combine into ${item.into.name}`}
              </Button>
            </span>
          </li>
        ))}
        {list.map((section) => {
          const resources = section.proposals.flatMap((proposal) => proposal.resources);
          const projects = section.proposals.length;
          return (
            <li key={section.id} className={styles.row}>
              <ServiceIcon id={section.provider} size={18} />
              <span className={styles.text}>
                <span>
                  {serviceName(section.provider)}
                  {section.label ? ` · ${section.label}` : ''}
                </span>
                <span className={styles.meta}>
                  {projects} {projects === 1 ? 'project' : 'projects'} ·{' '}
                  {resources
                    .slice(0, 4)
                    .map((resource) => resource.name)
                    .join(', ')}
                  {resources.length > 4 ? ` and ${String(resources.length - 4)} more` : ''}
                </span>
              </span>
              <span className={styles.actions}>
                <Button
                  variant="primary"
                  onClick={() => {
                    setOpen(true);
                  }}
                >
                  Review and add
                </Button>
              </span>
            </li>
          );
        })}
      </ul>
      <ErrorMessage error={combine.error} />
      {open && (
        <FoundDialog
          found={data}
          onClose={() => {
            setOpen(false);
          }}
          onConnect={
            onConnect
              ? (provider) => {
                  setOpen(false);
                  onConnect(provider);
                }
              : undefined
          }
        />
      )}
    </section>
  );
}
