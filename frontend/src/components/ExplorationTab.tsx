import { useState } from 'react';
import { Activity, GitFork, Search } from 'lucide-react';
import { api } from '../api';
import { formatDate, statusLabel } from '../format';
import Graph from '../graph/Graph';
import type { Action } from '../hooks/useAction';
import type { Edge, GraphData, Node, Overlap, Scan } from '../types';
import { EntityTable } from './EntityTable';
import { Inspector } from './Inspector';

type Props = {
  investigationId: string;
  canWrite: boolean;
  scans: Scan[];
  scan: Scan | undefined;
  graph: GraphData | null;
  action: Action;
  onScanCreated: (scan: Scan) => void;
  onSelectScan: (id: string) => void;
  onGraph: (graph: GraphData) => void;
  onOpenInvestigation: (id: string) => void;
};

export function ExplorationTab({
  investigationId,
  canWrite,
  scans,
  scan,
  graph,
  action,
  onScanCreated,
  onSelectScan,
  onGraph,
  onOpenInvestigation,
}: Props) {
  const [selected, setSelected] = useState<Node | Edge | null>(null);
  const [proof, setProof] = useState<unknown>(null);
  const [overlaps, setOverlaps] = useState<Overlap[]>([]);
  const { busy, run, setError } = action;

  const select = (item: Node | Edge) => {
    setSelected(item);
    setProof(null);
    setOverlaps([]);
    if ('data' in item)
      api<Overlap[]>(`/entities/${item.id}/overlaps`)
        .then(setOverlaps)
        .catch((failure: Error) => setError(failure.message));
  };
  const reloadGraph = (query = '') =>
    run(async () => {
      if (scan) onGraph(await api<GraphData>(`/scans/${scan.id}/graph${query}`));
    });

  return (
    <>
      <section className="scan-panel">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const form = new FormData(event.currentTarget);
            void run(async () => {
              const created = await api<Scan>(`/investigations/${investigationId}/scans`, 'POST', {
                seed: form.get('seed'),
                max_requests: Number(form.get('budget')),
                idempotency_key: crypto.randomUUID(),
              });
              onScanCreated(created);
            });
          }}
        >
          <label className="seed">
            <Search size={17} />
            <input
              name="seed"
              required
              aria-label="Compte ou dépôt GitHub"
              placeholder="github.com/compte/depot ou compte"
              disabled={!canWrite}
            />
          </label>
          <label className="budget">
            Budget
            <select name="budget" defaultValue="12" disabled={!canWrite}>
              <option value="6">6 requêtes</option>
              <option value="12">12 requêtes</option>
              <option value="24">24 requêtes</option>
            </select>
          </label>
          <button disabled={busy || !canWrite}>
            <Activity size={16} />
            Lancer un scan
          </button>
        </form>
        <p>Collecte publique bornée · Aucun code exécuté · Les données existantes restent consultables</p>
      </section>
      <section className="explorer">
        <div className="explorer-toolbar">
          <div>
            <span className="eyebrow">VUE DU SCAN</span>
            <select
              aria-label="Scan affiché"
              value={scan?.id ?? ''}
              onChange={(event) => {
                setSelected(null);
                setProof(null);
                onSelectScan(event.target.value);
              }}
            >
              {scans.length === 0 && <option>Aucun scan</option>}
              {scans.map((row) => (
                <option key={row.id} value={row.id}>
                  {row.seed} · {formatDate(row.requested_at)} · {statusLabel[row.status]}
                </option>
              ))}
            </select>
          </div>
          {scan && <span className={'badge ' + scan.status}>{statusLabel[scan.status]}</span>}
          <div className="graph-counts">
            <strong>{graph?.nodes.length ?? 0}</strong> entités <span>·</span>{' '}
            <strong>{graph?.edges.length ?? 0}</strong> liens
          </div>
        </div>
        <div className="graph-layout">
          <div className="graph-stage">
            {graph && graph.nodes.length > 0 ? (
              <Graph data={graph} onSelect={select} selectedId={selected?.id} />
            ) : (
              <div className="empty">
                <GitFork size={38} />
                <h3>
                  {scan?.status === 'running'
                    ? 'La collecte démarre…'
                    : 'Le graphe attend ses premières observations'}
                </h3>
                <p>Lancez un scan ou importez un snapshot historique depuis la CLI.</p>
              </div>
            )}
            <div className="legend">
              <span>
                <i className="account" />
                Compte
              </span>
              <span>
                <i className="repo" />
                Dépôt
              </span>
              <span>
                <i className="issue" />
                Issue / PR
              </span>
            </div>
          </div>
          <Inspector
            selected={selected}
            proof={proof}
            overlaps={overlaps}
            onClose={() => setSelected(null)}
            onShowProof={(item) =>
              void run(async () => setProof(await api(`/evidence/${item.evidence_hash}`)))
            }
            onNeighborhood={(node) => void reloadGraph(`?focus=${node.id}`)}
            onOpenInvestigation={onOpenInvestigation}
          />
        </div>
        <footer className="graph-footer">
          <span className="dot" />
          {graph?.truncated
            ? 'Vue limitée : utilisez le filtre ou ouvrez un voisinage.'
            : 'Vue des observations collectées, sans garantie d’exhaustivité.'}
          <button className="text-button" disabled={!scan} onClick={() => void reloadGraph()}>
            Réinitialiser la vue
          </button>
        </footer>
      </section>
      {scan?.error && <p className="notice">{scan.error}</p>}
      <EntityTable nodes={graph?.nodes ?? []} onSelect={select} />
    </>
  );
}
