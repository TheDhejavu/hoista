import { describe, expect, it } from 'vitest';
import { parseEnv } from './PasteVariablesDialog';

describe('parseEnv', () => {
  it('reads a .env file the way people write them', () => {
    const { values, bad } = parseEnv(
      '# keys\nexport API_KEY="abc=123"\nMODE=live\n\nNAME=\'with spaces\'\n',
    );
    expect(values).toEqual({ API_KEY: 'abc=123', MODE: 'live', NAME: 'with spaces' });
    expect(bad).toEqual([]);
  });

  it('points at lines that are not NAME=value', () => {
    expect(parseEnv('GOOD=1\n2BAD=x\nnovalue\nEMPTY=').bad).toEqual([2, 3, 4]);
  });
});
