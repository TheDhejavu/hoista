/**
 * A change Hoista sent to a provider but didn't hear back about. The person
 * checks the provider and says whether it happened; until then Hoista won't
 * make the same change again, so nothing is created twice.
 */
import { useState } from 'react';
import type { AttentionItem } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useCheckOperation } from '@ui/api/projects';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { timeAgo } from '@ui/lib/format';

type Operation = NonNullable<AttentionItem['operation']>;

export function UnclearDialog({
  operation,
  onClose,
}: {
  operation: Operation;
  onClose: () => void;
}) {
  const check = useCheckOperation();
  const [answer, setAnswer] = useState<boolean | null>(null);
  const provider = serviceName(operation.service);
  const send = (happened: boolean): void => {
    setAnswer(happened);
    check.mutate({ id: operation.id, happened }, { onSuccess: onClose });
  };

  return (
    <Dialog
      title={`Did this happen at ${provider}?`}
      eyebrow={`Started ${timeAgo(operation.started_at)}`}
      icon={<ServiceIcon id={operation.service} size={18} />}
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" disabled={check.isPending} onClick={onClose}>
            Later
          </Button>
          <Button
            busy={check.isPending && answer === false}
            disabled={check.isPending}
            onClick={() => {
              send(false);
            }}
          >
            It didn’t happen
          </Button>
          <Button
            variant="primary"
            busy={check.isPending && answer === true}
            disabled={check.isPending}
            onClick={() => {
              send(true);
            }}
          >
            It happened
          </Button>
        </>
      }
    >
      <DialogText>
        <p>
          Hoista asked {provider} for: <strong>{operation.target}</strong>. It didn’t hear back, so
          it may have happened.
        </p>
        <p>
          Look in {provider}. Until you answer, Hoista won’t make the same change again, so nothing
          is created twice.
        </p>
        <ErrorMessage error={check.error} />
      </DialogText>
    </Dialog>
  );
}
