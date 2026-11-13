/** A failed Rust command, as `{ code, message }` like the engine's own errors. */
export interface Failure {
  code: string;
  message: string;
}

export function asFailure(error: unknown): Failure {
  if (typeof error === 'object' && error !== null && 'code' in error && 'message' in error) {
    const { code, message } = error as Record<string, unknown>;
    if (typeof code === 'string' && typeof message === 'string') return { code, message };
  }
  return { code: 'app.failed', message: String(error) };
}
