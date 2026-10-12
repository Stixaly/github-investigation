import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Action } from '../hooks/useAction';
import { TriageTab } from './TriageTab';

const action = (): Action => ({ busy: false, error: '', setError: vi.fn(), run: async (task) => task() });

const item = {
  id: 't1',
  state: 'new',
  note: '',
  entity_id: 'e1',
  entity_kind: 'Repository',
  entity_label: 'demo-bob/observatory',
  list_id: 'l1',
  list_name: 'Démo',
  list_version: 2,
  engine_version: 1,
  rule_id: 'observatoire',
  review_priority: 'high',
  hits: [{ field: 'repository.name', term: 'observatory', start: 0, end: 11, excerpt: 'observatory' }],
  exclusions: [],
  not_evaluated: ['repository.readme'],
  evidence_hash: 'h1',
  observed_at: '2026-10-01T00:00:00Z',
  evaluated_at: '2026-10-12T00:00:00Z',
  updated_at: '2026-10-12T00:00:00Z',
  updated_by: null,
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('TriageTab', () => {
  it('explains each match and records the decision with its note', async () => {
    const fetch = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) =>
      init?.method === 'PUT'
        ? Response.json({ id: 't1', state: 'dismissed' })
        : Response.json({
            items: [item],
            counts: { new: 1, in_review: 0, retained: 0, dismissed: 3 },
            page: 1,
            page_size: 50,
            notice: 'Une correspondance indique un sujet à examiner.',
          }),
    );
    vi.stubGlobal('fetch', fetch);
    render(<TriageTab investigationId="i1" canWrite action={action()} />);
    expect(await screen.findByRole('heading', { name: 'demo-bob/observatory' })).toBeInTheDocument();
    expect(screen.getByText('Priorité haute')).toBeInTheDocument();
    expect(screen.getByText(/Non collecté : repository.readme/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Tous/ })).toHaveTextContent('4');
    expect(String(fetch.mock.calls[0][0])).toBe('/api/v1/investigations/i1/triage?page=1&state=new');
    await userEvent.type(screen.getByLabelText('Note sur demo-bob/observatory'), 'Fork de démo');
    await userEvent.click(screen.getByRole('button', { name: 'Écarté' }));
    const decision = fetch.mock.calls.find(([, init]) => init?.method === 'PUT');
    expect(String(decision?.[0])).toBe('/api/v1/triage/t1');
    expect(JSON.parse(String(decision?.[1]?.body))).toEqual({ state: 'dismissed', note: 'Fork de démo' });
  });

  it('lets readers look without deciding', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_input: RequestInfo | URL) =>
        Response.json({ items: [item], counts: { new: 1 }, page: 1, page_size: 50, notice: '' }),
      ),
    );
    render(<TriageTab investigationId="i1" canWrite={false} action={action()} />);
    expect(await screen.findByRole('heading', { name: 'demo-bob/observatory' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Écarté' })).not.toBeInTheDocument();
  });
});
