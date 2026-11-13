/**
 * "Ask your agent": the things people usually want done to a project, as prompts
 * to paste into their coding agent. Anything risky comes back here as an
 * approval before it happens.
 */
import { Dialog, DialogText } from './Dialog';
import { AgentPrompt } from './AgentPrompt';
import styles from './AskAgentDialog.module.css';

export interface SuggestedPrompt {
  prompt: string;
  hint?: string;
}

export function AskAgentDialog({
  title,
  prompts,
  onClose,
}: {
  title: string;
  prompts: SuggestedPrompt[];
  onClose: () => void;
}) {
  return (
    <Dialog title={title} onClose={onClose}>
      <DialogText>
        <p>
          Paste one of these into Claude Code, Cursor, ChatGPT or any agent connected to Hoista.
          Anything that deletes or changes access waits for your OK first.
        </p>
      </DialogText>
      <div className={styles.list}>
        {prompts.map((item) => (
          <AgentPrompt key={item.prompt} prompt={item.prompt} hint={item.hint} />
        ))}
      </div>
    </Dialog>
  );
}
