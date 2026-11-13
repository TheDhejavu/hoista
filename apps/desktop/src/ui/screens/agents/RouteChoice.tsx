import { type ReactNode } from 'react';
import { Cloud, Monitor } from 'lucide-react';
import type { Settings } from '@shared/api';
import { useUpdateSettings } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import styles from './RouteChoice.module.css';

export type Route = 'computer' | 'remote';

/** This computer through a tunnel, or a remote: two choices, one picked. */
export function RouteChoice({
  clientName,
  route,
  onRoute,
  settings,
}: {
  clientName: string;
  route: Route | null;
  onRoute: (route: Route) => void;
  settings: Settings;
}) {
  const update = useUpdateSettings();
  const mcpOff = !settings.agents.mcp_http;
  return (
    <div
      className={styles.routes}
      role="radiogroup"
      aria-label={`How ${clientName} reaches Hoista`}
    >
      <span className={styles.label}>How should {clientName} reach Hoista?</span>
      <RouteCard
        selected={route === 'computer'}
        onSelect={() => {
          onRoute('computer');
        }}
        icon={<Monitor size={20} aria-hidden="true" />}
        title="This computer"
        text="Through a tunnel you run, such as cloudflared. Works while this computer is awake."
      >
        {mcpOff && route === 'computer' && (
          <span className={styles.off}>
            MCP over HTTP is off.
            <Button
              busy={update.isPending}
              busyLabel="Turning on"
              onClick={(event) => {
                event.stopPropagation();
                update.mutate({ agents: { mcp_http: true } });
              }}
            >
              Turn it on
            </Button>
          </span>
        )}
      </RouteCard>
      <RouteCard
        selected={route === 'remote'}
        onSelect={() => {
          onRoute('remote');
        }}
        icon={<Cloud size={20} aria-hidden="true" />}
        title="A remote"
        text={
          settings.agents.remote.target === 'this_computer'
            ? 'Hoista in your own cloud account. Works when this computer is off.'
            : 'Your remote. Works when this computer is off.'
        }
      />
      <ErrorMessage error={update.error} />
    </div>
  );
}

function RouteCard({
  selected,
  onSelect,
  icon,
  title,
  text,
  children,
}: {
  selected: boolean;
  onSelect: () => void;
  icon: ReactNode;
  title: string;
  text: string;
  children?: ReactNode;
}) {
  return (
    <div
      role="radio"
      aria-checked={selected}
      tabIndex={0}
      className={cx(styles.route, selected && styles.routeOn)}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key === ' ' || event.key === 'Enter') {
          event.preventDefault();
          onSelect();
        }
      }}
    >
      <span className={cx(styles.radio, selected && styles.radioOn)} aria-hidden="true" />
      <span className={styles.routeText}>
        <b>{title}</b>
        <span>{text}</span>
        {children}
      </span>
      <span className={styles.routeIcon}>{icon}</span>
    </div>
  );
}
