import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Edge, Node } from '../types';
import { Inspector } from './Inspector';

const handlers = () => ({
  onClose: vi.fn(),
  onShowProof: vi.fn(),
  onNeighborhood: vi.fn(),
  onOpenInvestigation: vi.fn(),
});

const node: Node = {
  id: 'n1',
  kind: 'Repository',
  label: 'alice/old',
  evidence_hash: 'h1',
  observed_at: null,
  time_precision: 'day',
  provisional: true,
  data: {},
};

const edge: Edge = {
  id: 'e1',
  kind: 'STARRED',
  source_id: 'a',
  target_id: 'b',
  evidence_hash: 'h2',
  observed_at: '2026-10-01T00:00:00Z',
  source_event_at: '2026-01-01T00:00:00Z',
  time_precision: 'instant',
};

describe('Inspector', () => {
  it('reminds that a link is not an attribution', () => {
    render(<Inspector selected={null} proof={null} overlaps={[]} {...handlers()} />);
    expect(screen.getByText(/ne constitue pas une attribution/)).toBeInTheDocument();
  });

  it('flags provisional identities and opens neighborhoods and overlaps', async () => {
    const actions = handlers();
    render(
      <Inspector
        selected={node}
        proof={null}
        overlaps={[{ id: 'i2', title: 'Autre dossier' }]}
        {...actions}
      />,
    );
    expect(screen.getByRole('heading', { name: 'alice/old' })).toBeInTheDocument();
    expect(screen.getByText(/Identité historique provisoire/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Ouvrir le voisinage' }));
    expect(actions.onNeighborhood).toHaveBeenCalledWith(node);
    await userEvent.click(screen.getByRole('button', { name: 'Autre dossier' }));
    expect(actions.onOpenInvestigation).toHaveBeenCalledWith('i2');
  });

  it('shows a relation with its event date and evidence', async () => {
    const actions = handlers();
    render(
      <Inspector selected={edge} proof={{ payload: { type: 'WatchEvent' } }} overlaps={[]} {...actions} />,
    );
    expect(screen.getByRole('heading', { name: 'STARRED' })).toBeInTheDocument();
    expect(screen.getByText('Événement')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Ouvrir le voisinage' })).not.toBeInTheDocument();
    expect(screen.getByText(/WatchEvent/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Consulter la preuve/ }));
    expect(actions.onShowProof).toHaveBeenCalledWith(edge);
  });

  it('collects the relations of an entity and compares its neighbours with a retained one', async () => {
    const actions = {
      ...handlers(),
      onCollect: vi.fn(),
      onPin: vi.fn(),
      onCommon: vi.fn(),
      onSelectEntity: vi.fn(),
    };
    const account: Node = { ...node, id: 'n2', kind: 'Account', label: 'demo-alice', provisional: false };
    const { rerender } = render(<Inspector selected={account} proof={null} overlaps={[]} {...actions} />);
    await userEvent.click(screen.getByRole('button', { name: /Collecter ses relations/ }));
    expect(actions.onCollect).toHaveBeenCalledWith(account);
    await userEvent.click(screen.getByRole('button', { name: 'Retenir pour comparer' }));
    expect(actions.onPin).toHaveBeenCalledWith(account);
    rerender(<Inspector selected={account} proof={null} overlaps={[]} pinned={account} {...actions} />);
    expect(screen.getByText(/Entité retenue/)).toBeInTheDocument();
    rerender(
      <Inspector
        selected={node}
        proof={null}
        overlaps={[]}
        pinned={account}
        common={{
          a: 'n2',
          b: 'n1',
          common: [
            {
              id: 'r1',
              kind: 'Repository',
              label: 'demo-lab/observatory',
              with_a: ['CONTRIBUTED_TO'],
              with_b: ['OWNS'],
            },
          ],
          notice: 'Un voisin commun n’établit pas un lien.',
        }}
        {...actions}
      />,
    );
    // A provisional historical identity cannot seed a new scan.
    expect(screen.queryByRole('button', { name: /Collecter ses relations/ })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Voisins communs avec demo-alice' }));
    expect(actions.onCommon).toHaveBeenCalledWith(node);
    expect(screen.getByRole('heading', { name: '1 voisin commun' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /demo-lab\/observatory/ }));
    expect(actions.onSelectEntity).toHaveBeenCalledWith('r1');
  });
});
