import type { Domain } from '@shared/api';
import { useCheckDomain } from '@ui/api/projects';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { useCopy } from '@ui/lib/useCopy';
import styles from './FinishCard.module.css';
import shared from './domains.module.css';

export function FinishCard({ projectId, domain }: { projectId: string; domain: Domain }) {
  const { copied, copy } = useCopy();
  const check = useCheckDomain(projectId);
  const setup = domain.setup;
  if (!setup) return null;
  const where = setup.dns_host ? `${setup.dns_host}’s` : 'the domain’s';
  const records = setup.records;

  return (
    <aside className={shared.card} aria-labelledby="finish-heading">
      <h2 id="finish-heading">Finish {domain.name}</h2>
      <p className={shared.cardText}>
        {records.length > 0
          ? `Add this record where ${where} DNS is managed. Hoista checks every minute.`
          : `Set these nameservers${setup.registrar ? ` in ${setup.registrar}` : ''}. Hoista checks every few minutes.`}
      </p>
      <div className={shared.recordFrame}>
        {records.length > 0 ? (
          <table className={shared.records}>
            <thead>
              <tr>
                <th scope="col">Type</th>
                <th scope="col">Name</th>
                <th scope="col">Value</th>
              </tr>
            </thead>
            <tbody>
              {records.map((record) => (
                <tr key={`${record.type}-${record.name}`}>
                  <td>{record.type}</td>
                  <td>{record.name}</td>
                  <td className={shared.value} title={record.value}>
                    {record.value}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <ol className={styles.nameservers}>
            {setup.nameservers.map((server) => (
              <li key={server}>{server}</li>
            ))}
          </ol>
        )}
      </div>
      <div className={styles.cardActions}>
        <Button
          large
          onClick={() => {
            copy(
              records.length > 0
                ? records
                    .map((record) => `${record.type} ${record.name} ${record.value}`)
                    .join('\n')
                : setup.nameservers.join('\n'),
            );
          }}
        >
          {copied ? 'Copied' : records.length > 0 ? 'Copy record' : 'Copy nameservers'}
        </Button>
        {setup.help_url && (
          <Button
            large
            onClick={() => {
              if (setup.help_url) void openExternal(setup.help_url);
            }}
          >
            Show me where ↗
          </Button>
        )}
        <Button
          variant="ghost"
          large
          disabled={check.isPending}
          onClick={() => {
            check.mutate(domain.name);
          }}
        >
          {check.isPending ? 'Checking' : 'Check now'}
        </Button>
      </div>
    </aside>
  );
}
