import { ArrowUpRight, CircleDot, FolderSearch, Radar, X } from 'lucide-react';
import { formatDate } from '../format';
import type { CommonNeighbours, Edge, Node, Overlap } from '../types';

type Props = {
  selected: Node | Edge | null;
  proof: unknown;
  overlaps: Overlap[];
  onClose: () => void;
  onShowProof: (item: Node | Edge) => void;
  onNeighborhood: (node: Node) => void;
  onOpenInvestigation: (id: string) => void;
  /** Launches a scan seeded by the selected account or repository; absent for readers. */
  onCollect?: (node: Node) => void;
  /** Entity kept aside to look for neighbours shared with the next selection. */
  pinned?: Node | null;
  onPin?: (node: Node) => void;
  onCommon?: (node: Node) => void;
  common?: CommonNeighbours | null;
  onSelectEntity?: (id: string) => void;
};

const isNode = (item: Node | Edge): item is Node => 'data' in item;

/** Kinds that GitHub can be asked about again, as a new scan seed. */
const COLLECTABLE = ['Account', 'Repository'];

/** Side panel: date, precision and evidence of the selected entity or relation. */
export function Inspector({
  selected,
  proof,
  overlaps,
  onClose,
  onShowProof,
  onNeighborhood,
  onOpenInvestigation,
  onCollect,
  pinned = null,
  onPin,
  onCommon,
  common = null,
  onSelectEntity,
}: Props) {
  if (!selected) {
    return (
      <aside className="inspector">
        <span className="inspector-symbol">
          <CircleDot size={25} />
        </span>
        <h3>Lire une relation</h3>
        <p>Sélectionnez un compte, un dépôt ou un lien pour consulter sa source et sa date d’observation.</p>
        <div className="notice">
          Un lien observé établit une interaction. Il ne constitue pas une attribution.
        </div>
      </aside>
    );
  }
  const node = isNode(selected) ? selected : null;
  const shared = common?.common.length ?? 0;
  return (
    <aside className="inspector">
      <div className="inspector-title">
        <p className="eyebrow">{selected.kind}</p>
        <button className="icon" aria-label="Fermer le détail" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <h3>{node ? node.label : selected.kind}</h3>
      <dl>
        <dt>Observation</dt>
        <dd>{formatDate(selected.observed_at)}</dd>
        <dt>Précision</dt>
        <dd>{selected.time_precision}</dd>
        {!isNode(selected) && selected.source_event_at && (
          <>
            <dt>Événement</dt>
            <dd>{formatDate(selected.source_event_at)}</dd>
          </>
        )}
      </dl>
      {node?.provisional && (
        <p className="notice">Identité historique provisoire : ID GitHub absent de la source.</p>
      )}
      <button className="secondary" onClick={() => onShowProof(selected)}>
        Consulter la preuve <ArrowUpRight size={15} />
      </button>
      {node && (
        <button className="secondary" onClick={() => onNeighborhood(node)}>
          Ouvrir le voisinage
        </button>
      )}
      {node && onCollect && COLLECTABLE.includes(node.kind) && !node.provisional && (
        <button className="secondary" onClick={() => onCollect(node)}>
          <Radar size={15} /> Collecter ses relations
        </button>
      )}
      {node && onPin && pinned?.id === node.id && (
        <p className="notice">
          Entité retenue : sélectionnez-en une autre pour chercher leurs voisins communs.
        </p>
      )}
      {node && onPin && pinned?.id !== node.id && (
        <button className="secondary" onClick={() => onPin(node)}>
          Retenir pour comparer
        </button>
      )}
      {node && pinned && pinned.id !== node.id && onCommon && (
        <button className="secondary" onClick={() => onCommon(node)}>
          Voisins communs avec {pinned.label}
        </button>
      )}
      {common && (
        <>
          <h4>
            {shared} voisin{shared > 1 ? 's' : ''} commun{shared > 1 ? 's' : ''}
          </h4>
          {common.common.map((entity) => (
            <button className="overlap" key={entity.id} onClick={() => onSelectEntity?.(entity.id)}>
              {entity.label}
              <small>
                {entity.with_a.join(', ')} · {entity.with_b.join(', ')}
              </small>
            </button>
          ))}
          <p className="notice">{common.notice}</p>
        </>
      )}
      {overlaps.length > 0 && (
        <>
          <h4>Recoupements accessibles</h4>
          {overlaps.map((overlap) => (
            <button className="overlap" key={overlap.id} onClick={() => onOpenInvestigation(overlap.id)}>
              <FolderSearch size={14} />
              {overlap.title}
            </button>
          ))}
        </>
      )}
      {proof !== null && <pre className="proof">{JSON.stringify(proof, null, 2)}</pre>}
    </aside>
  );
}
