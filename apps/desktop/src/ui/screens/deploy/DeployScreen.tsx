/**
 * One deploy at `/deploys/:deployId`. Shows its progress while it runs, then
 * the result: live, or what went wrong and how to fix it. Engine events keep
 * it up to date, so there is no polling.
 */
import { useDeploy } from '@ui/api/deploys';
import { Page } from '@ui/components/Page';
import { QueryState } from '@ui/components/QueryState';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { DeployDoneView } from './DeployDoneView';
import { DeployCancelledView } from './DeployCancelledView';
import { DeployFailedView } from './DeployFailedView';
import { DeployingView } from './DeployingView';
import { DeploySkeleton } from './DeployResources';

export function DeployScreen() {
  const deployId = useRequiredParam('deployId');
  const deploy = useDeploy(deployId);

  return (
    <Page>
      <QueryState query={deploy} skeleton={<DeploySkeleton />}>
        {(data) => {
          switch (data.status) {
            case 'queued':
            case 'running':
              return <DeployingView deploy={data} />;
            case 'failed':
              return <DeployFailedView deploy={data} />;
            case 'cancelled':
              return <DeployCancelledView deploy={data} />;
            case 'live':
            case 'rolled_back':
              return <DeployDoneView deploy={data} />;
          }
        }}
      </QueryState>
    </Page>
  );
}
