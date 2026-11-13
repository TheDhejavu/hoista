import { describe, expect, it } from 'vitest';
import { parseServerSentEvent, toResult } from './wire';

describe('parseServerSentEvent', () => {
  it('reads a daemon activity entry', () => {
    const dto = {
      id: '42',
      at: '2026-11-03T00:00:00Z',
      actor: 'agent-1',
      project_id: 'moodboard',
      subject_id: 'deploy-7',
      kind: 'deploy.step',
      summary: 'Building.',
    };
    const message = `id: 42\nevent: deploy.step\ndata: ${JSON.stringify(dto)}`;
    expect(parseServerSentEvent(message)).toEqual(dto);
  });

  it('ignores keep-alives, broken JSON, error frames and non-events', () => {
    expect(parseServerSentEvent(': keep-alive')).toBeNull();
    expect(parseServerSentEvent('data: {not json')).toBeNull();
    expect(
      parseServerSentEvent(
        'event: stream.error\ndata: {"code":"state.unavailable","message":"Reconnect."}',
      ),
    ).toBeNull();
    expect(parseServerSentEvent('data: {"type":"agent.connected"}')).toBeNull();
  });
});

describe('toResult', () => {
  it('passes data through on success', () => {
    expect(toResult(200, '{"name":"moodboard"}')).toEqual({
      ok: true,
      data: { name: 'moodboard' },
    });
  });

  it('keeps the engine’s plain-English error', () => {
    expect(
      toResult(404, '{"code":"project_not_found","message":"That project no longer exists."}'),
    ).toEqual({
      ok: false,
      error: { code: 'project_not_found', message: 'That project no longer exists.' },
    });
  });

  it('falls back to a generic error for a body that is not an error object', () => {
    expect(toResult(500, 'Internal Server Error')).toEqual({
      ok: false,
      error: { code: 'http_500', message: 'The Hoista engine returned an error.' },
    });
  });
});
