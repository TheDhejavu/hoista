import { describe, expect, it } from 'vitest';
import type { ImportProposal, Resource } from '@engine-api';
import { projectName, toFound } from './imports';

function resource(id: string, name: string): Resource {
  return {
    id,
    project_id: null,
    kind: 'service',
    provider: 'railway',
    account_id: 'rw',
    provider_ref: `project/${id}`,
    name,
    region: null,
    status: 'unknown',
    mode: 'watching',
    url: null,
    group: null,
  };
}

describe('found on your providers', () => {
  it('says which workspace and project each resource was found in', () => {
    const api = { ...resource('a', 'api'), group: 'portfolio' };
    const found = toFound(
      [api],
      [],
      [],
      [
        {
          id: 'rw',
          provider: 'railway',
          label: 'Sam’s workspace',
          external_id: 'w1',
          scopes: [],
          status: 'connected',
          connected_at: '2026-11-03T10:00:00Z',
          last_discovery: null,
          credential: null,
          in_1password: false,
        },
      ],
    );
    expect(found.resources[0]).toMatchObject({
      account: { id: 'rw', label: 'Sam’s workspace' },
      group: 'portfolio',
    });
  });

  it('offers groups only when every resource is still free, worded by evidence', () => {
    const proposals: ImportProposal[] = [
      {
        id: 'p1',
        label: 'notes',
        resource_ids: ['a', 'b'],
        evidence: { kind: 'shared_name', value: 'notes' },
        ambiguous: false,
      },
      {
        id: 'p2',
        label: 'gone',
        resource_ids: ['a', 'taken'],
        evidence: { kind: 'shared_name', value: 'x' },
        ambiguous: false,
      },
    ];
    const found = toFound([resource('a', 'notes-api'), resource('b', 'notes-db')], proposals);
    expect(found.groups).toEqual([
      {
        id: 'p1',
        label: 'notes',
        resource_ids: ['a', 'b'],
        reason: 'Same name: notes',
        ambiguous: false,
      },
    ]);
    expect(found.resources.map((resource) => resource.name)).toEqual(['notes-api', 'notes-db']);
  });
});

describe('project names from provider names', () => {
  it('turns display names into names the engine accepts', () => {
    expect(projectName('Postgres')).toBe('postgres');
    expect(projectName('My API (prod)')).toBe('my-api-prod');
    expect(projectName('  ---  ')).toBe('project');
    expect(projectName('Café Ünïcode')).toBe('cafe-unicode');
  });
});
