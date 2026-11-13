/**
 * Remove from project: takes one resource out of a watched project in Hoista. The
 * resource stays on its provider, untouched, and goes back under Found on your
 * providers. Nothing changes at the provider and it can be put straight back, so it
 * happens at once with an Undo in the note. Removing the last one removes
 * the project too, so that one asks first, in a dialog over whatever is open.
 */
import { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { Resource } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useAddToProject, useStopWatchingResource } from '@ui/api/imports';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { showToast } from '@ui/lib/toast';
import { useRequiredParam } from '@ui/lib/useRequiredParam';

export function RemoveFromProject({
  resource,
  projectName,
  last,
  large = false,
  onRemoved,
  opened,
  onClosed,
}: {
  resource: Pick<Resource, 'id' | 'name' | 'provider'>;
  projectName: string;
  /** The app's only resource: removing it removes the app too. */
  last: boolean;
  large?: boolean;
  /** After removing, when the app is still there. */
  onRemoved?: () => void;
  /** Opened from elsewhere, such as a row's menu: no button of its own. */
  opened?: boolean;
  onClosed?: () => void;
}) {
  const [own, setOwn] = useState(false);
  const controlled = opened !== undefined;
  const open = controlled ? opened : own;
  const setOpen = (next: boolean) => {
    if (controlled) {
      if (!next) onClosed?.();
    } else setOwn(next);
  };
  const remove = useStopWatchingResource();
  const addBack = useAddToProject();
  const projectId = useRequiredParam('projectId');
  const navigate = useNavigate();
  const provider = serviceName(resource.provider);

  // Not the last resource: remove at once, and offer to put it back.
  const removeNow = (): void => {
    remove.mutate(resource.id, {
      onSuccess: () => {
        onRemoved?.();
        showToast(`Removed ${resource.name} from ${projectName}`, {
          action: {
            label: 'Undo',
            onClick: () => {
              addBack.mutate({ projectId, resourceIds: [resource.id] });
            },
          },
        });
      },
      onError: (error) => {
        showToast(`${resource.name} wasn’t removed. ${error.message}`, { tone: 'error' });
      },
    });
  };
  // Opened from a row's menu: the choice was the click, so it happens now.
  const started = useRef(false);
  useEffect(() => {
    if (!controlled || !opened || last) {
      started.current = false;
      return;
    }
    if (started.current) return;
    started.current = true;
    removeNow();
    onClosed?.();
  });

  return (
    <>
      {!controlled && (
        <Button
          variant="ghost"
          large={large}
          busy={remove.isPending}
          busyLabel="Removing"
          onClick={() => {
            if (last) setOpen(true);
            else removeNow();
          }}
        >
          Remove from project
        </Button>
      )}
      {open && last && (
        <Dialog
          title={`Remove ${resource.name} from ${projectName}?`}
          icon={<ServiceIcon id={resource.provider} size={18} />}
          onClose={() => {
            setOpen(false);
          }}
          actions={
            <>
              <Button
                onClick={() => {
                  setOpen(false);
                }}
              >
                Cancel
              </Button>
              <Button
                variant="danger"
                busy={remove.isPending}
                busyLabel="Removing"
                onClick={() => {
                  remove.mutate(resource.id, {
                    onSuccess: () => {
                      setOpen(false);
                      showToast(`Removed ${resource.name}. It’s under Found on your providers.`);
                      void navigate('/');
                    },
                  });
                }}
              >
                Remove and close the project
              </Button>
            </>
          }
        >
          <DialogText>
            <p>
              {resource.name} stays on {provider}, untouched. Hoista stops showing it in{' '}
              {projectName} and puts it back under Found on your providers, where you can add it to
              any project.
            </p>
            <p>
              It’s the only resource in {projectName}, so {projectName} goes away from Hoista too.
            </p>
          </DialogText>
          <ErrorMessage error={remove.error} />
        </Dialog>
      )}
    </>
  );
}
