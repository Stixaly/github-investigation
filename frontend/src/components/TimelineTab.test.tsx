import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Action } from '../hooks/useAction';
import type { Scan } from '../types';
import { TimelineTab } from './TimelineTab';

const scan = (id: string, status: Scan['status']): Scan => ({
  id,
  seed: 'demo-lab/observatory',
  source: 'github',
  status,
  requested_at: '2026-10-01T00:00:00Z',
  coverage: {},
  requests_used: 3,
  max_requests: 12,
});

const action = (): Action => ({
  busy: false,
  error: '',
  setError: vi.fn(),
  run: async (task) => task(),
});

describe('TimelineTab', () => {
  it('compares a scan with the previous one and never calls a gap a deletion', async () => {
    const fetch = vi.fn(async (_input: RequestInfo | URL) =>
      Response.json({
        newly_observed: 2,
        not_reobserved: 1,
        shared: 5,
        notice: 'Non réobservé ne signifie pas supprimé.',
      }),
    );
    vi.stubGlobal('fetch', fetch);
    render(
      <TimelineTab
        scans={[scan('new', 'succeeded'), scan('old', 'succeeded')]}
        canWrite
        action={action()}
        onExplore={vi.fn()}
        onScansChanged={vi.fn()}
      />,
    );
    const compare = screen.getAllByRole('button', { name: 'Comparer au précédent' });
    expect(compare).toHaveLength(1);
    await userEvent.click(compare[0]);
    expect(fetch.mock.calls[0][0]).toBe('/api/v1/scans/old/compare/new');
    expect(await screen.findByText('Non réobservé ne signifie pas supprimé.')).toBeInTheDocument();
  });

  it('offers cancellation for pending scans to writers only', () => {
    const scans = [scan('a', 'running'), scan('b', 'failed')];
    const { rerender } = render(
      <TimelineTab scans={scans} canWrite action={action()} onExplore={vi.fn()} onScansChanged={vi.fn()} />,
    );
    expect(screen.getAllByRole('button', { name: 'Annuler' })).toHaveLength(1);
    rerender(
      <TimelineTab
        scans={scans}
        canWrite={false}
        action={action()}
        onExplore={vi.fn()}
        onScansChanged={vi.fn()}
      />,
    );
    expect(screen.queryByRole('button', { name: 'Annuler' })).not.toBeInTheDocument();
  });
});
