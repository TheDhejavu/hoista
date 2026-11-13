/**
 * `meta.toast` on a mutation: the confirmation note to show once it
 * succeeds. main.tsx shows it for every mutation that sets one.
 */
import '@tanstack/react-query';

declare module '@tanstack/react-query' {
  interface Register {
    mutationMeta: {
      toast?: string | ((data: unknown, variables: unknown) => string);
    };
  }
}
