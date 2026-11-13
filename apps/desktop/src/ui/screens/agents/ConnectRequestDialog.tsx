/**
 * Something wants to connect: a web agent through MCP over HTTP, or a coding
 * tool on this computer, each showing a code. The person chooses what it may
 * do, lets it in, or says no. Letting a web agent in asks for Touch ID or the
 * password on this computer.
 */
import { useState } from 'react';
import type { AccessLevel, ConnectionRequest, Decision } from '@shared/api';
import { useAnswerConnectionRequest } from '@ui/api/agents';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { Fact, Facts } from '@ui/components/Facts';
import { SelectField } from '@ui/components/Select';
import { ACCESS_CHOICES, ACCESS_HINTS, accessLabel } from '@ui/lib/agentAccess';
import { showToast } from '@ui/lib/toast';

export function ConnectRequestDialog({
  request,
  onClose,
}: {
  request: ConnectionRequest;
  onClose: () => void;
}) {
  const answer = useAnswerConnectionRequest();
  const local = request.via === 'this_computer';
  const [access, setAccess] = useState<AccessLevel>(request.access);

  function decide(decision: Decision['decision']): void {
    answer.mutate(
      { requestId: request.id, decision, access },
      {
        onSuccess: () => {
          showToast(
            decision === 'allow'
              ? `${request.claimed_name} is connected · ${accessLabel(access)}`
              : `Didn’t allow ${request.claimed_name}`,
          );
          onClose();
        },
      },
    );
  }

  return (
    <Dialog
      title={`${request.claimed_name} wants to connect`}
      eyebrow="New connection"
      onClose={onClose}
      actions={
        <>
          <Button
            disabled={answer.isPending}
            onClick={() => {
              decide('decline');
            }}
          >
            Don’t allow
          </Button>
          <Button
            variant="accent"
            busy={answer.isPending}
            busyLabel={local ? 'Allowing' : 'Waiting for Touch ID'}
            onClick={() => {
              decide('allow');
            }}
          >
            {local ? `Allow ${request.claimed_name}` : 'Allow with Touch ID'}
          </Button>
        </>
      }
    >
      <Facts>
        <Fact term="Says it is">
          {request.claimed_name}
          <span>The name it gave. Hoista cannot verify it.</span>
        </Fact>
        <Fact term="Came from">
          {local ? 'A tool on this computer' : 'From the web, through your tunnel'}
        </Fact>
        {request.code && (
          <Fact term="Code">
            <span className="mono">{request.code}</span>
            <span>
              Hoista’s page in your browser shows the same code. If it doesn’t match, don’t allow
              it.
            </span>
          </Fact>
        )}
      </Facts>
      <SelectField
        label="What it may do"
        value={access}
        options={ACCESS_CHOICES}
        onChange={setAccess}
      />
      <DialogText>
        <p>
          {`${ACCESS_HINTS[access]}. You can change this, and limit it to some projects, any time in Agents.`}
          {local && request.replaces
            ? ` It replaces the earlier ${request.claimed_name} connection on this computer, which stops working.`
            : ''}
          {local
            ? ''
            : ` Only allow this if you just added Hoista’s address in ${request.claimed_name} yourself. If you didn’t, don’t allow it, and forget the address in Settings → Agents.`}
        </p>
        <ErrorMessage error={answer.error} />
      </DialogText>
    </Dialog>
  );
}
