import { describe, expect, it } from 'vitest';
import { toAgentCall } from './agents';

describe('toAgentCall', () => {
  it('shows what the agent sent and how it ended', () => {
    const call = toAgentCall({
      tool: 'run_provider_operation',
      sent: { resource_id: 'p1', name: 'serviceDelete', args: {} },
      result: { outcome: 'pending', message: null, approval_id: 'ap_1' },
    });
    expect(call.sent).toContain('run_provider_operation');
    expect(call.sent).toBe(
      'run_provider_operation {\n  resource_id: "p1"\n  name: "serviceDelete"\n  args: {}\n}',
    );
    expect(call.status).toBe('Waiting for your approval');
    expect(call.happened).toEqual([]);
    expect(call.approval_id).toBe('ap_1');
  });

  it('keeps the refusal message', () => {
    const call = toAgentCall({
      tool: 'set_secret',
      sent: { name: 'API_KEY', value: '•••• (not recorded)' },
      result: { outcome: 'rejected', message: 'That project does not exist.' },
    });
    expect(call.status).toBe('Refused');
    expect(call.happened).toEqual(['That project does not exist.']);
  });

  it('is empty for calls recorded before details were kept', () => {
    expect(toAgentCall(null)).toEqual({
      sent: null,
      status: null,
      happened: [],
      approval_id: null,
    });
  });
});
