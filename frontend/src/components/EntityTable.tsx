import { useState } from 'react';
import { formatDate } from '../format';
import type { Node } from '../types';

type Props = { nodes: Node[]; onSelect: (node: Node) => void };

/** Accessible list of the entities in the current view, filterable by label. */
export function EntityTable({ nodes, onSelect }: Props) {
  const [filter, setFilter] = useState('');
  const needle = filter.toLowerCase();
  return (
    <section className="entity-section">
      <div className="section-title">
        <h2>Entités observées</h2>
        <input
          aria-label="Filtrer la liste des entités"
          placeholder="Filtrer la liste…"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
      </div>
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Entité</th>
              <th>Type</th>
              <th>Observation</th>
              <th>Identité</th>
            </tr>
          </thead>
          <tbody>
            {nodes
              .filter((node) => node.label.toLowerCase().includes(needle))
              .map((node) => (
                <tr key={node.id}>
                  <td>
                    <button className="text-button" onClick={() => onSelect(node)}>
                      {node.label}
                    </button>
                  </td>
                  <td>{node.kind}</td>
                  <td>{formatDate(node.observed_at)}</td>
                  <td>{node.provisional ? 'Provisoire' : 'ID stable'}</td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
