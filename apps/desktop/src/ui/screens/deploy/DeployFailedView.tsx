/**
 * A deploy that failed: the steps it got through, what went wrong in plain
 * English, and the build output. When the cause is a missing secret, the
 * key is pasted right here (secrets never go through an AI chat), then your
 * agent deploys again. For Hoista's own deploys the live site is unchanged; a
 * provider's own deploy only says what the provider reported.
 */
import { useState } from 'react';
import type { Deploy } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useProject, useSetSecret } from '@ui/api/projects';
import { Button, ButtonLink } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { timeAgo } from '@ui/lib/format';
import { useCopy } from '@ui/lib/useCopy';
import { DeployHead, DeployLog, DeployPill, StepChips } from './DeployResources';
import styles from './DeployFailedView.module.css';

export function DeployFailedView({ deploy }: { deploy: Deploy }) {
  const project = useProject(deploy.project_id);
  const name = project.data?.project.name ?? deploy.project_id;
  const provider = project.data?.project.providers[0];
  const missing = deploy.failure?.missing_secrets ?? [];
  const who = deploy.started_by.kind === 'person' ? 'you' : deploy.started_by.name;

  return (
    <div className={styles.page}>
      <DeployHead
        deploy={deploy}
        title={`${name} didn’t deploy`}
        mascot="failed"
        pill={<DeployPill tone="bad">Deploy failed</DeployPill>}
        sub={
          deploy.started_by.kind === 'system'
            ? `${deploy.started_by.name} reported it failed · ${timeAgo(deploy.created_at)}`
            : `Nothing changed on your live site · by ${who} ${timeAgo(deploy.created_at)}`
        }
        actions={
          <ButtonLink to={`/projects/${deploy.project_id}/deploys`} large>
            All deploys of {name}
          </ButtonLink>
        }
      />
      <StepChips steps={deploy.steps} />
      <div className={styles.columns}>
        <div className={styles.card}>
          <h2>What went wrong</h2>
          <p className={styles.explain}>
            {missing[0] ? (
              <>
                The service needs <span className="mono">{missing[0]}</span>, and it is not set.
              </>
            ) : (
              (deploy.failure?.message ?? 'The deploy stopped before it finished.')
            )}
          </p>
          {missing.length === 0 && deploy.failure?.fix && (
            <p className={styles.explain}>{deploy.failure.fix}</p>
          )}
          {missing.length > 0 ? (
            <MissingSecrets
              projectId={deploy.project_id}
              projectName={name}
              names={missing}
              providerName={provider ? serviceName(provider) : 'your provider'}
            />
          ) : (
            <AskAgain
              projectName={name}
              prompt={`Using Hoista, read the build and runtime logs of ${name}'s failed deploy ${deploy.version}, find the cause, fix it in the code, deploy the fix and confirm it's live.`}
            />
          )}
        </div>
        <DeployLog deployId={deploy.id} height={230} />
      </div>
    </div>
  );
}

/** "Ask your agent: “Deploy … again with Hoista”", copied on click. */
export function AskAgain({ projectName, prompt: given }: { projectName: string; prompt?: string }) {
  const { copied, copy } = useCopy();
  const prompt = given ?? `Deploy ${projectName} again with Hoista`;
  return (
    <button
      type="button"
      className={styles.ask}
      title="Copy this for your agent"
      onClick={() => {
        copy(prompt);
      }}
    >
      {copied ? (
        'Copied. Paste it to your agent.'
      ) : (
        <>
          Ask your agent: <b>“{prompt}”</b>
        </>
      )}
    </button>
  );
}

/** Asks for the secrets the deploy needed. Values go to the provider and are never stored here. */
function MissingSecrets({
  projectId,
  projectName,
  names,
  providerName,
}: {
  projectId: string;
  projectName: string;
  names: string[];
  providerName: string;
}) {
  const setSecret = useSetSecret(projectId);
  const [values, setValues] = useState<Record<string, string>>({});
  const [saved, setSaved] = useState(false);
  const complete = names.every((name) => values[name]?.trim());

  async function save(): Promise<void> {
    for (const name of names) {
      await setSecret.mutateAsync({ name, value: values[name] ?? '' });
    }
    // The values were only needed for these requests.
    setValues({});
    setSaved(true);
  }

  return (
    <>
      {names.map((name) => (
        <div key={name} className={styles.field}>
          <label htmlFor={`secret-${name}`} className={styles.label}>
            {name}
          </label>
          <input
            id={`secret-${name}`}
            type="password"
            autoComplete="off"
            spellCheck={false}
            className={styles.input}
            placeholder={name.startsWith('RESEND') ? 're_…' : 'Paste value'}
            value={values[name] ?? ''}
            onChange={(event) => {
              setValues({ ...values, [name]: event.target.value });
            }}
          />
        </div>
      ))}
      <p className={styles.hint}>
        Sent straight to {providerName} as a secret. Never stored here, never shown to your agent.
      </p>
      <div className={styles.actions}>
        <Button
          variant="accent"
          large
          disabled={setSecret.isPending || (!complete && !saved)}
          onClick={() => {
            void save();
          }}
        >
          {saved ? 'Key saved' : setSecret.isPending ? 'Saving' : 'Save key'}
        </Button>
        <span className={styles.then}>then</span>
        <AskAgain projectName={projectName} />
      </div>
      <ErrorMessage error={setSecret.error} />
    </>
  );
}
