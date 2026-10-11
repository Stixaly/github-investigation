import { ArrowUpRight, CircleDot, FolderSearch, X } from 'lucide-react';
import { formatDate } from '../format';
import type { Edge, Node, Overlap } from '../types';

type Props = {
  selected: Node | Edge | null;
  proof: unknown;
  overlaps: Overlap[];
  onClose: () => void;
  onShowProof: (item: Node | Edge) => void;
  onNeighborhood: (node: Node) => void;
  onOpenInvestigation: (id: string) => void;
};

const isNode = (item: Node | Edge): item is Node => 'data' in item;

/** Side panel: date, precision and evidence of the selected entity or relation. */
export function Inspector({
  selected,
  proof,
  overlaps,
  onClose,
  onShowProof,
  onNeighborhood,
  onOpenInvestigation,
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
  return (
    <aside className="inspector">
      <div className="inspector-title">
        <p className="eyebrow">{selected.kind}</p>
        <button className="icon" aria-label="Fermer le détail" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <h3>{isNode(selected) ? selected.label : selected.kind}</h3>
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
      {isNode(selected) && selected.provisional && (
        <p className="notice">Identité historique provisoire : ID GitHub absent de la source.</p>
      )}
      <button className="secondary" onClick={() => onShowProof(selected)}>
        Consulter la preuve <ArrowUpRight size={15} />
      </button>
      {isNode(selected) && (
        <button className="secondary" onClick={() => onNeighborhood(selected)}>
          Ouvrir le voisinage
        </button>
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
