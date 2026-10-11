import { useState } from 'react';
import { Activity, GitFork, Search } from 'lucide-react';
import { api } from '../api';
import { formatDate, statusLabel } from '../format';
import Graph from '../graph/Graph';
import type { Action } from '../hooks/useAction';
import type { CommonNeighbours, Edge, GraphData, Node, Overlap, Scan } from '../types';
import { ALL_SCANS } from '../types';
import { EntityTable } from './EntityTable';
import { Inspector } from './Inspector';

type Props = {
  investigationId: string;
  canWrite: boolean;
  scans: Scan[];
  scan: Scan | undefined;
  /** Shown view: a scan id, or `ALL_SCANS` for the whole investigation. */
  view: string;
  /** API path of the shown graph, empty while there is nothing to show. */
  graphPath: string;
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
  view,
  graphPath,
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
  const [pinned, setPinned] = useState<Node | null>(null);
  const [common, setCommon] = useState<CommonNeighbours | null>(null);
  const { busy, run, setError } = action;

  const select = (item: Node | Edge) => {
    setSelected(item);
    setProof(null);
    setOverlaps([]);
    setCommon(null);
    if ('data' in item)
      api<Overlap[]>(`/entities/${item.id}/overlaps`)
        .then(setOverlaps)
        .catch((failure: Error) => setError(failure.message));
  };
  const reloadGraph = (query = '') =>
    run(async () => {
      if (graphPath) onGraph(await api<GraphData>(graphPath + query));
    });
  const launch = (seed: string, budget: number) =>
    run(async () => {
      const created = await api<Scan>(`/investigations/${investigationId}/scans`, 'POST', {
        seed,
        max_requests: budget,
        idempotency_key: crypto.randomUUID(),
      });
      onScanCreated(created);
    });
  const selectEntity = (id: string) => {
    const node = graph?.nodes.find((candidate) => candidate.id === id);
    if (node) select(node);
    else void reloadGraph(`?focus=${id}`);
  };

  return (
    <>
      <section className="scan-panel">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const form = new FormData(event.currentTarget);
            void launch(String(form.get('seed')), Number(form.get('budget')));
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
            <span className="eyebrow">VUE</span>
            <select
              aria-label="Scan affiché"
              value={view}
              onChange={(event) => {
                setSelected(null);
                setProof(null);
                setCommon(null);
                onSelectScan(event.target.value);
              }}
            >
              {scans.length === 0 && <option>Aucun scan</option>}
              {scans.length > 0 && (
                <option value={ALL_SCANS}>
                  Tout le dossier · {scans.length} scan{scans.length > 1 ? 's' : ''}
                </option>
              )}
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
            onCollect={canWrite ? (node) => void launch(node.label, 12) : undefined}
            pinned={pinned}
            onPin={setPinned}
            onCommon={(node) =>
              pinned &&
              void run(async () =>
                setCommon(
                  await api<CommonNeighbours>(
                    `/investigations/${investigationId}/common?a=${pinned.id}&b=${node.id}`,
                  ),
                ),
              )
            }
            common={common}
            onSelectEntity={selectEntity}
          />
        </div>
        <footer className="graph-footer">
          <span className="dot" />
          {graph?.truncated
            ? 'Vue limitée : utilisez le filtre ou ouvrez un voisinage.'
            : 'Vue des observations collectées, sans garantie d’exhaustivité.'}
          <button className="text-button" disabled={!graphPath} onClick={() => void reloadGraph()}>
            Réinitialiser la vue
          </button>
        </footer>
      </section>
      {scan?.error && <p className="notice">{scan.error}</p>}
      <EntityTable nodes={graph?.nodes ?? []} onSelect={select} />
    </>
  );
}
