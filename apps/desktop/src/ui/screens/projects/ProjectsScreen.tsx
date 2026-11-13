/**
 * Projects: what needs you across the top, then every project as a table or a grid,
 * with dropdowns to filter, sort, switch view and pick columns. Before any
 * provider is connected it's the first-run screen; until the Get started
 * checklist is done and hidden it shows the projects as cards beside it.
 */
import { Package, Search } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import type {
  ProjectList,
  ProjectListItem,
  ProjectListQuery,
  ProviderId,
  Status,
} from '@shared/api';
import { PROVIDER_IDS, serviceName } from '@shared/providers';
import { useProjects } from '@ui/api/projects';
import { ALL_ENVIRONMENTS, useCurrentEnvironment } from '@ui/api/environments';
import { CopyPromptButton } from '@ui/components/AgentPrompt';
import { Page } from '@ui/components/Page';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { EmptyState } from '@ui/components/EmptyState';
import { PillSelect, type Option } from '@ui/components/Select';
import { useStoredChoice, useStoredFilter, useStoredOption } from '@ui/lib/storedChoice';
import { useBills } from '@ui/api/accounts';
import { billSummary } from '@ui/lib/bills';
import { formatCost, plural, shortDate } from '@ui/lib/format';
import { ProjectsDialogs, type ProjectsDialog } from './ProjectsDialogs';
import { ProjectsGrid } from './ProjectsGrid';
import { ProjectsTable } from './ProjectsTable';
import { ColumnsMenu, DEFAULT_COLUMNS, isColumnList, type ColumnId } from './columns';
import { EmptyProjects } from './EmptyProjects';
import { FirstProjects } from './FirstProjects';
import { AGENT_PROMPT, SETUP_STEPS, useSetupDone } from './GetStarted';
import { FoundOnProviders } from './FoundOnProviders';
import { NeedsStrip } from './NeedsStrip';
import styles from './ProjectsScreen.module.css';

const GET_STARTED_HIDDEN = 'hoista.apps.getStartedHidden';

type Sort = NonNullable<ProjectListQuery['sort']>;

type View = 'table' | 'grid';

/** `null` in a filter means "any". */
const STATUS_CHOICES: Option<Status | null>[] = [
  { value: null, label: 'All' },
  { value: 'live', label: 'Live' },
  { value: 'failing', label: 'Failing' },
  { value: 'deploying', label: 'Deploying' },
  { value: 'sleeping', label: 'Sleeping' },
  { value: 'idle', label: 'Idle' },
  { value: 'paused', label: 'Paused' },
  { value: 'stopped', label: 'Stopped' },
  { value: 'not_deployed', label: 'Not deployed' },
];

const PROVIDER_CHOICES: Option<ProviderId | null>[] = [
  { value: null, label: 'All' },
  ...PROVIDER_IDS.map((id) => ({ value: id, label: serviceName(id) })),
];

const SORT_CHOICES: Option<Sort>[] = [
  { value: 'last_deploy', label: 'Last deploy' },
  { value: 'requests', label: 'Requests' },
  { value: 'name', label: 'Name' },
];

const VIEW_CHOICES: Option<View>[] = [
  { value: 'table', label: 'Table' },
  { value: 'grid', label: 'Grid' },
];

function isView(value: unknown): value is View {
  return value === 'table' || value === 'grid';
}

export function ProjectsScreen() {
  const [search, setSearch] = useState('');
  // Filters stay as you left them, like the view and columns.
  const [status, setStatus] = useStoredFilter<Status>('projects.status', null);
  const [provider, setProvider] = useStoredFilter<ProviderId>('projects.provider', null);
  const [sort, setSort] = useStoredOption<Sort>('projects.sort', 'last_deploy', SORT_CHOICES);
  const [view, setView] = useStoredChoice<View>('hoista.apps.view', 'table', isView);
  const [columns, setColumns] = useStoredChoice<ColumnId[]>(
    'hoista.apps.columns',
    DEFAULT_COLUMNS,
    isColumnList,
  );
  const [dialog, setDialog] = useState<ProjectsDialog>(null);

  const projects = useProjects({
    q: search,
    status: status ?? undefined,
    provider: provider ?? undefined,
    sort,
  });

  // Every project in every environment: an empty environment isn't a fresh install.
  const everything = useProjects({ environment: ALL_ENVIRONMENTS });
  const shown = useCurrentEnvironment();
  // Check the total, not the filtered rows, so an empty search still shows the table.
  const isEmpty = everything.data?.totals.projects === 0;
  const [startedHidden, setStartedHidden] = useStoredChoice<boolean>(
    GET_STARTED_HIDDEN,
    false,
    (value): value is boolean => typeof value === 'boolean',
  );
  const done = useSetupDone();
  // The first projects and the checklist show until setup is done, then once more:
  // leaving Projects after that puts them away on their own.
  const isSettingUp = done < SETUP_STEPS || !startedHidden;
  const finished = done === SETUP_STEPS && !startedHidden;
  useEffect(() => {
    if (!finished) return undefined;
    return () => {
      try {
        window.localStorage.setItem(GET_STARTED_HIDDEN, 'true');
      } catch {
        // Not remembered: it shows once more next time.
      }
    };
  }, [finished]);

  return (
    <>
      {isEmpty ? (
        <EmptyProjects
          setOpen={setDialog}
          found={
            <FoundOnProviders
              onConnect={(provider) => {
                setDialog({ kind: 'connect', provider });
              }}
            />
          }
        />
      ) : isSettingUp && projects.data && projects.data.totals.projects > 0 ? (
        <FirstProjects
          projects={projects.data.items}
          onHide={() => {
            setStartedHidden(true);
          }}
          onConnect={(provider) => {
            setDialog({ kind: 'connect', provider });
          }}
        />
      ) : (
        <Page>
          <h1 className="visually-hidden">Projects</h1>
          <NeedsStrip />
          <FoundOnProviders
            onConnect={(provider) => {
              setDialog({ kind: 'connect', provider });
            }}
          />

          <div className={styles.toolbar}>
            <label className={styles.search}>
              <Search size={14} aria-hidden="true" />
              <input
                type="search"
                aria-label="Filter projects"
                placeholder="Filter projects"
                value={search}
                onChange={(event) => {
                  setSearch(event.target.value);
                }}
              />
            </label>
            <PillSelect
              label="Status"
              value={status}
              options={STATUS_CHOICES}
              onChange={setStatus}
            />
            <PillSelect
              label="Provider"
              value={provider}
              options={PROVIDER_CHOICES}
              onChange={setProvider}
            />
            <span className={styles.spacer} />
            <PillSelect label="View" value={view} options={VIEW_CHOICES} onChange={setView} />
            {view === 'table' && <ColumnsMenu value={columns} onChange={setColumns} />}
            <PillSelect label="Sort" value={sort} options={SORT_CHOICES} onChange={setSort} />
            <CopyPromptButton label="Ask your agent" prompt={AGENT_PROMPT} />
          </div>

          <QueryState query={projects} skeleton={<SkeletonRows count={5} columns={4} />}>
            {(data) => (
              <div className={styles.list}>
                {data.totals.projects === 0 ? (
                  <EmptyState icon={<Package />} title="No projects yet">
                    Projects you bring in with <code>hoist import</code> or deploy with{' '}
                    <code>hoist deploy</code> show up here in{' '}
                    {shown.environment?.name ?? 'this environment'}. Or ask your agent to{' '}
                    <code>deploy this with Hoista</code>.
                  </EmptyState>
                ) : (
                  <ProjectsList projects={data.items} view={view} columns={columns} />
                )}
                <Totals totals={data.totals} shown={data.items.length} />
              </div>
            )}
          </QueryState>
        </Page>
      )}
      {/* Kept in the same place either way, so a dialog survives the first projects arriving. */}
      <ProjectsDialogs
        open={dialog}
        onClose={() => {
          setDialog(null);
        }}
      />
    </>
  );
}

function ProjectsList({
  projects,
  view,
  columns,
}: {
  projects: ProjectListItem[];
  view: View;
  columns: ColumnId[];
}) {
  if (projects.length === 0) {
    return (
      <div className={styles.empty}>
        No projects match these filters. <Link to="/">Show every project</Link>
      </div>
    );
  }
  return view === 'grid' ? (
    <ProjectsGrid projects={projects} />
  ) : (
    <ProjectsTable projects={projects} columns={columns} />
  );
}

/** One quiet line under the list, so the toolbar keeps to filters and actions. */
function Totals({ totals, shown }: { totals: ProjectList['totals']; shown: number }) {
  const filtered = shown < totals.projects ? `Showing ${shown} of ${totals.projects} · ` : '';
  // Only what providers actually billed, per account: Hoista doesn't estimate.
  const bill = billSummary(useBills().data);
  return (
    <p className={styles.totals}>
      {filtered}
      {plural(totals.projects, 'project')} on {plural(totals.providers, 'provider')}.
      {bill && (
        <>
          {' '}
          Used on {bill.providers} this period
          {bill.ends ? ` (ends ${shortDate(bill.ends)})` : ''}:{' '}
          <Link to="/billing" className="mono">
            {formatCost(bill.total)}
          </Link>
          .
        </>
      )}
    </p>
  );
}
