import { describe, expect, it } from 'vitest';
import type { EngineEvent } from '@shared/api';
import { keysAffectedBy } from './engine';
import { agentKeys, projectKeys, deployKeys } from './keys';

function event(kind: string, project_id: string | null = null): EngineEvent {
  return {
    id: '1',
    at: '2026-11-03T00:00:00Z',
    actor: 'engine',
    project_id,
    subject_id: null,
    kind,
    summary: '',
  };
}

describe('keysAffectedBy', () => {
  it('refreshes a deploy and its project when a step changes', () => {
    expect(keysAffectedBy(event('deploy.step', 'moodboard'))).toEqual([
      deployKeys.recent,
      projectKeys.lists(),
      projectKeys.detail('moodboard'),
      projectKeys.attention,
    ]);
  });

  it('refreshes approvals and attention for approval events', () => {
    expect(keysAffectedBy(event('approval.created'))).toContainEqual(agentKeys.approvals);
  });

  it('refreshes projects after a change at a provider and after reading it again', () => {
    expect(keysAffectedBy(event('provider.operation'))).toContainEqual(projectKeys.all);
    expect(keysAffectedBy(event('resource.refreshed'))).toContainEqual(projectKeys.all);
  });

  it('ignores kinds it does not know, instead of failing', () => {
    expect(keysAffectedBy(event('something.new'))).toEqual([]);
  });
});
