/**
 * A commit message as Git writes it: the first line is the subject, and
 * whatever follows a blank line is the body.
 */
export function splitCommit(message: string | null | undefined): {
  subject: string;
  body: string;
} {
  const text = (message ?? '').replace(/\r\n/g, '\n').trim();
  const end = text.indexOf('\n');
  if (end === -1) return { subject: text, body: '' };
  return { subject: text.slice(0, end).trim(), body: text.slice(end + 1).trim() };
}

/** How many lines a commit's body has beyond its subject, 0 for none. */
export function extraLines(message: string | null | undefined): number {
  const { body } = splitCommit(message);
  return body ? body.split('\n').filter((line) => line.trim() !== '').length : 0;
}
