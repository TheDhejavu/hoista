/**
 * How a project or resource status is worded and coloured, as in the designs.
 */
import type { Status } from '@shared/api';
import type { Tone } from '@ui/components/Dot';

interface StatusLook {
  label: string;
  tone: Tone;
}

/** Label and colour for a status. `idleDays` adds the count to "Idle". */
export function statusLook(status: Status, idleDays: number | null = null): StatusLook {
  switch (status) {
    case 'live':
      return { label: 'Live', tone: 'good' };
    case 'failing':
      return { label: 'Deploy failed', tone: 'bad' };
    case 'sleeping':
      return { label: 'Sleeping', tone: 'hollow' };
    case 'idle':
      return { label: idleDays ? `Idle ${idleDays} days` : 'Idle', tone: 'warn' };
    case 'paused':
      return { label: 'Paused', tone: 'warn' };
    case 'deploying':
      return { label: 'Deploying', tone: 'warn' };
    case 'not_deployed':
      return { label: 'Not deployed', tone: 'off' };
    case 'stopped':
      return { label: 'Stopped', tone: 'off' };
    case 'unknown':
      return { label: 'Checking', tone: 'off' };
  }
}
