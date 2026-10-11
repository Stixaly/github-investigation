import { describe, expect, it } from 'vitest';
import type { Node } from '../types';
import { colorOf, icon, kindOf, kinds, relationLabel, search } from './model';

const node = (id: string, kind: string, label: string, data: Record<string, unknown> = {}): Node => ({
  id,
  kind,
  label,
  data,
  evidence_hash: 'h',
  observed_at: null,
  time_precision: 'instant',
  provisional: false,
});

describe('graph model', () => {
  it('shows organizations apart from accounts', () => {
    expect(kindOf(node('1', 'Account', 'demo-lab', { type: 'Organization' }))).toBe('Organization');
    expect(kindOf(node('2', 'Account', 'alice', { type: 'User' }))).toBe('Account');
    expect(kindOf(node('3', 'PullRequest', '#1'))).toBe('PullRequest');
    expect(kindOf(node('4', 'Unknown', '?'))).toBe('Account');
    expect(colorOf(node('5', 'Repository', 'a/b'))).toBe(kinds.Repository.color);
  });

  it('translates relations and keeps unknown kinds readable', () => {
    expect(relationLabel('STARRED')).toBe('a ajouté une étoile');
    expect(relationLabel('NEW_KIND')).toBe('NEW_KIND');
  });

  it('draws icons as inline SVG in the kind color', () => {
    const uri = icon('Repository');
    expect(uri.startsWith('data:image/svg+xml;utf8,')).toBe(true);
    expect(decodeURIComponent(uri)).toContain(kinds.Repository.color);
  });

  it('searches visible kinds only, case-insensitively, at most eight results', () => {
    const nodes = [
      node('a', 'Account', 'Demo-Alice'),
      node('r', 'Repository', 'demo-lab/observatory'),
      ...Array.from({ length: 10 }, (_, index) => node(`x${index}`, 'Issue', `#${index} demo`)),
    ];
    expect(search(nodes, '  ALICE ', []).map((match) => match.id)).toEqual(['a']);
    expect(search(nodes, 'demo', ['Issue']).map((match) => match.id)).toEqual(['a', 'r']);
    expect(search(nodes, 'demo', [])).toHaveLength(8);
    expect(search(nodes, '   ', [])).toEqual([]);
  });
});
