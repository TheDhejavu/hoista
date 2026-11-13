/**
 * The dialogs the Projects screen opens: connect a provider, or import from
 * another computer. The screen owns them, so a
 * dialog opened from the empty state stays open when the first projects arrive
 * and the table replaces it.
 */
import type { ProviderId } from '@shared/api';
import { ConnectServiceDialog } from '@ui/screens/accounts/ConnectDialog';
import { ImportTransferDialog } from '@ui/screens/settings/transfer/ImportTransferDialog';

export type ProjectsDialog = { kind: 'connect'; provider: ProviderId } | { kind: 'import' } | null;

export function ProjectsDialogs({ open, onClose }: { open: ProjectsDialog; onClose: () => void }) {
  switch (open?.kind) {
    case 'connect':
      return <ConnectServiceDialog service={open.provider} onClose={onClose} />;
    case 'import':
      return <ImportTransferDialog onClose={onClose} onImported={onClose} />;
    default:
      return null;
  }
}
