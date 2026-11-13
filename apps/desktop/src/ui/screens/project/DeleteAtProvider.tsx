/**
 * Delete at the provider: removes a resource for good, at the provider itself (a
 * project, service, Worker, bucket or sending domain), after the person types
 * its name. Offered only where the provider lets Hoista delete. Asks in a
 * dialog over whatever is open; the app goes too when it was the last thing
 * in it.
 */
import { useId, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { Resource } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useProviderAction } from '@ui/api/deploys';
import { useCan } from '@ui/api/providers';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { resourceKindLabel } from './resourceKinds';
import styles from './DeleteAtProvider.module.css';

export function DeleteAtProvider({
  resource,
  last,
  opened,
  onClosed,
}: {
  resource: Pick<Resource, 'id' | 'name' | 'provider' | 'kind'>;
  /** The app's only resource: deleting it removes the app from Hoista too. */
  last: boolean;
  /** Opened from elsewhere, such as a row's menu: no button of its own. */
  opened?: boolean;
  onClosed?: () => void;
}) {
  const can = useCan();
  const [own, setOwn] = useState(false);
  const controlled = opened !== undefined;
  const open = controlled ? opened : own;
  const [typed, setTyped] = useState('');
  const setOpen = (next: boolean) => {
    if (!next) setTyped('');
    if (controlled) {
      if (!next) onClosed?.();
    } else setOwn(next);
  };
  const remove = useProviderAction();
  const navigate = useNavigate();
  const inputId = useId();
  const provider = serviceName(resource.provider);
  if (!can(resource.provider, 'delete')) return null;

  return (
    <>
      {!controlled && (
        <Button
          variant="quietDanger"
          onClick={() => {
            setTyped('');
            setOpen(true);
          }}
        >
          Delete
        </Button>
      )}
      {open && (
        <Dialog
          title={`Delete ${resource.name} on ${provider}?`}
          icon={<ServiceIcon id={resource.provider} size={18} />}
          onClose={() => {
            if (!remove.isPending) setOpen(false);
          }}
          actions={
            <>
              <Button
                disabled={remove.isPending}
                onClick={() => {
                  setOpen(false);
                }}
              >
                Cancel
              </Button>
              <Button
                variant="danger"
                disabled={typed !== resource.name}
                busy={remove.isPending}
                busyLabel="Deleting"
                onClick={() => {
                  remove.mutate(
                    {
                      resourceId: resource.id,
                      action: 'remove_service',
                      confirmationName: typed,
                    },
                    {
                      onSuccess: () => {
                        setOpen(false);
                        if (last) void navigate('/');
                      },
                    },
                  );
                }}
              >
                Delete on {provider}
              </Button>
            </>
          }
        >
          <DialogText>
            {provider} deletes this {resourceKindLabel(resource.kind).toLowerCase()} and everything
            in it. This can’t be undone, and anything using it stops working.
            {last
              ? ' It’s the last thing in this project, so the project goes from Hoista too.'
              : ''}
          </DialogText>
          <label htmlFor={inputId} className={styles.label}>
            Type <span className="mono">{resource.name}</span> to confirm
          </label>
          <input
            id={inputId}
            className={styles.input}
            autoComplete="off"
            spellCheck={false}
            autoFocus
            value={typed}
            onChange={(event) => {
              setTyped(event.target.value);
            }}
          />
          <ErrorMessage error={remove.error} />
        </Dialog>
      )}
    </>
  );
}
