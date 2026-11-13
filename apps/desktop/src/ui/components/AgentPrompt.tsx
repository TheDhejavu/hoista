/**
 * Something to say to your coding agent, with a Copy button.
 *
 * Hoista is driven by agents over MCP: instead of a button that does the work,
 * screens show the exact words to paste into Claude Code, Cursor or ChatGPT.
 * `AgentPrompt` is the full box; `CopyPromptButton` is the compact version
 * for rows and cards.
 */
import { Bot } from 'lucide-react';
import { useCopy } from '@ui/lib/useCopy';
import { Button } from './Button';
import styles from './AgentPrompt.module.css';

export function AgentPrompt({ prompt, hint }: { prompt: string; hint?: string }) {
  const { copied, copy } = useCopy();

  return (
    <div className={styles.box}>
      <Bot size={16} className={styles.icon} aria-hidden="true" />
      <div className={styles.text}>
        <q className={styles.prompt}>{prompt}</q>
        {hint && <span className={styles.hint}>{hint}</span>}
      </div>
      <Button
        aria-label={`Copy prompt: ${prompt}`}
        onClick={() => {
          copy(prompt);
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </Button>
    </div>
  );
}

/** A small button that copies a prompt for your agent. */
export function CopyPromptButton({
  prompt,
  label = 'Ask your agent',
  icon = true,
  strong = false,
}: {
  prompt: string;
  label?: string;
  /** Leave the bot icon out, for tight spots such as cards. */
  icon?: boolean;
  /** The main action where it sits, drawn as the accent button. */
  strong?: boolean;
}) {
  const { copied, copy } = useCopy();

  return (
    <Button
      {...(strong && { variant: 'accent' as const })}
      title="Copies a prompt to paste into your agent"
      onClick={() => {
        copy(prompt);
      }}
    >
      {icon && <Bot size={14} aria-hidden="true" />}
      {copied ? 'Prompt copied' : label}
    </Button>
  );
}
