/**
 * Puts the desktop's local API on top of `hoistd`.
 *
 * The screens were designed against their own presentation-shaped routes.
 * This adapter answers the routes whose
 * shape differs from the daemon's by calling the daemon and translating, and
 * passes every other request straight through. It never invents data: values
 * the engine has not observed stay null.
 */
import type { Engine } from '@shared/engine';
import { Router } from '@shared/router';
import { addAccountRoutes } from './accounts';
import { addAgentRoutes } from './agents';
import { addProjectRoutes } from './projects';
import { addAttentionRoutes } from './attention';
import { caller } from './call';
import { addDeployRoutes } from './deploys';
import { addImportRoutes } from './imports';
import { addProviderRoutes, providerReads } from './provider';
import { addLogRoutes } from './logs';
import { addOverviewRoutes } from './overview';
import { addResourceRoutes } from './resources';
import { addProfileRoutes } from './profile';
import { addServerRoutes } from './server';
import { addSettingsRoutes } from './settings';
import { withDefaults, type Desktop } from './desktop';
import { addTransferRoutes } from './transfer';

export type { Cli, Desktop } from './desktop';

export function createAdapter(daemon: Engine, given: Partial<Desktop> = {}): Engine {
  const desktop = withDefaults(daemon, given);
  const { local } = desktop;
  const router = new Router();
  const call = caller(daemon);
  // The profile is the person's, kept on this computer whichever Hoista is shown.
  addProfileRoutes(router, caller(local), desktop.userName);
  // Putting Hoista on a server is always done from this computer.
  addServerRoutes(router, caller(local), desktop.answersAt);
  addAccountRoutes(router, call, desktop.openPage);
  const reads = providerReads(call);
  addProjectRoutes(router, call, reads);
  addOverviewRoutes(router, call, reads);
  addDeployRoutes(router, call, reads);
  addSettingsRoutes(router, call, desktop, local === daemon ? call : caller(local));
  addAttentionRoutes(router, call);
  addImportRoutes(router, call);
  addResourceRoutes(router, call, reads);
  addLogRoutes(router, call, daemon.stream?.bind(daemon));
  addAgentRoutes(router, call);
  addProviderRoutes(router, call, reads);
  addTransferRoutes(router, call);

  return {
    request: (request) =>
      router.handles(request) ? router.dispatch(request, () => undefined) : daemon.request(request),
    subscribe: (listener) => daemon.subscribe(listener),
    ...(daemon.stream ? { stream: daemon.stream.bind(daemon) } : {}),
  };
}
