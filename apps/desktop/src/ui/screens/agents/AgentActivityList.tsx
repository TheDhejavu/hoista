/**
 * What agents did lately, newest first, on the Agents page: one line per
 * action, with reads by the same agent grouped into one line. The full
 * list with filters and details is on the Activity page.
 */
import { Link } from 'react-router-dom';
import type { AgentActivity } from '@shared/api';
import { useAgentActivity } from '@ui/api/agents';
import { useInEnvironment } from '@ui/lib/useInEnvironment';
import { AgentIcon } from '@ui/components/AgentIcon';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { activityText, shortAgo } from '@ui/lib/format';
import { reconnectPath } from '@shared/reconnect';
import styles from './AgentActivityList.module.css';
import { useProjectNames } from '@ui/lib/useProjectNames';

/** Lines shown here; the rest are on the Activity page. */
const SHOWN = 5;

interface Line {
  id: string;
  agent: string;
  text: string;
  detail: string | null;
  at: string;
  needsYou: boolean;
  to: string;
}

export function AgentActivityList() {
  const activity = useAgentActivity();
  const projectName = useProjectNames();
  const inEnvironment = useInEnvironment();

  return (
    <section className={styles.section} aria-labelledby="did-heading">
      <div className={styles.head}>
        <h2 id="did-heading">What agents did</h2>
        <span className={styles.faint}>Newest first · reads grouped</span>
        <Link to="/agents/activity" className={styles.all}>
          See all activity →
        </Link>
      </div>
      <QueryState
        query={activity}
        skeleton={
          <div className={styles.frame}>
            <SkeletonRows count={4} columns={3} />
          </div>
        }
      >
        {(data) =>
          data.items.length === 0 ? (
            <p className={styles.empty}>
              Nothing yet. Every tool call an agent makes shows up here.
            </p>
          ) : (
            <div className={styles.frame}>
              {grouped(
                data.items.filter((item) => inEnvironment(item.project_id)),
                projectName,
              )
                .slice(0, SHOWN)
                .map((line) => (
                  <Link key={line.id} to={line.to} className={styles.row}>
                    <AgentIcon name={line.agent} small />
                    <span className={styles.text}>
                      <b>{line.agent}</b> {line.text}
                      {line.detail && <span className={styles.muted}> · {line.detail}</span>}
                    </span>
                    {line.needsYou && <span className={styles.needs}>Waiting for you</span>}
                    <span className={styles.when}>{shortAgo(line.at).replace(' ago', '')}</span>
                  </Link>
                ))}
            </div>
          )
        }
      </QueryState>
    </section>
  );
}

/** One line per action; an agent's reads on the same day become one line. */
function grouped(items: AgentActivity[], projectName: (id: string | null) => string): Line[] {
  const lines: Line[] = [];
  const reads = new Map<string, { line: Line; count: number; projects: Set<string> }>();

  for (const item of items) {
    if (item.read_only) {
      const key = `${item.agent_id}-${new Date(item.at).toDateString()}`;
      const group = reads.get(key);
      if (group) {
        group.count += 1;
        if (item.project_id) group.projects.add(projectName(item.project_id));
        group.line.text = `read things ${group.count} times`;
        group.line.detail = [...group.projects].join(', ') || null;
        continue;
      }
      const line: Line = {
        id: item.id,
        agent: item.agent_name,
        text: activityText(item.summary, null),
        detail: item.project_id ? projectName(item.project_id) : null,
        at: item.at,
        needsYou: false,
        to: `/agents/activity?item=${item.id}`,
      };
      reads.set(key, {
        line,
        count: 1,
        projects: new Set(item.project_id ? [projectName(item.project_id)] : []),
      });
      lines.push(line);
      continue;
    }
    lines.push({
      id: item.id,
      agent: item.agent_name,
      text: activityText(item.summary, item.project_id ? projectName(item.project_id) : null),
      detail: item.waiting_on ? 'waiting for you' : item.result,
      at: item.at,
      needsYou: item.waiting_on !== null,
      to: item.waiting_on ? reconnectPath(item.waiting_on) : `/agents/activity?item=${item.id}`,
    });
  }
  return lines;
}
