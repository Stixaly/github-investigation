import { useState } from 'react';
import { Activity } from 'lucide-react';
import { api } from '../api';
import { formatDate, statusLabel } from '../format';
import type { Action } from '../hooks/useAction';
import { PENDING, type Comparison, type Scan } from '../types';

type Props = {
  scans: Scan[];
  canWrite: boolean;
  action: Action;
  onExplore: (scanId: string) => void;
  onScansChanged: () => Promise<void>;
};

export function TimelineTab({ scans, canWrite, action, onExplore, onScansChanged }: Props) {
  const [comparison, setComparison] = useState<Comparison | null>(null);
  const { run } = action;
  return (
    <section className="panel">
      <div className="section-title">
        <h2>Historique des scans</h2>
        <span>{scans.length} scan(s)</span>
      </div>
      <p>Comparez deux observations. Un lien non réobservé n’est pas nécessairement supprimé.</p>
      {scans.map((scan, index) => {
        const previous = scans[index + 1];
        return (
          <article className="scan-row" key={scan.id}>
            <div className="timeline-icon">
              <Activity size={18} />
            </div>
            <div className="scan-info">
              <h3>
                {scan.seed} <span className={'badge ' + scan.status}>{statusLabel[scan.status]}</span>
              </h3>
              <p>
                {formatDate(scan.requested_at)} · {scan.source} · {scan.requests_used}/{scan.max_requests}{' '}
                requêtes
              </p>
              <details>
                <summary>Couverture et limites</summary>
                <pre>{JSON.stringify(scan.coverage, null, 2)}</pre>
              </details>
              {scan.error && <p className="error">{scan.error}</p>}
            </div>
            <div className="row-actions">
              <button className="secondary" onClick={() => onExplore(scan.id)}>
                Explorer
              </button>
              {previous && (
                <button
                  className="secondary"
                  onClick={() =>
                    void run(async () =>
                      setComparison(await api<Comparison>(`/scans/${previous.id}/compare/${scan.id}`)),
                    )
                  }
                >
                  Comparer au précédent
                </button>
              )}
              {canWrite && PENDING.includes(scan.status) && (
                <button
                  className="secondary"
                  onClick={() =>
                    void run(async () => {
                      await api(`/scans/${scan.id}/cancel`, 'POST');
                      await onScansChanged();
                    })
                  }
                >
                  Annuler
                </button>
              )}
            </div>
          </article>
        );
      })}
      {comparison && (
        <div className="comparison">
          <h3>Différences entre observations</h3>
          <div>
            <strong>{comparison.newly_observed}</strong> nouveaux liens observés{' '}
            <strong>{comparison.not_reobserved}</strong> non réobservés <strong>{comparison.shared}</strong>{' '}
            partagés
          </div>
          <p>{comparison.notice}</p>
        </div>
      )}
    </section>
  );
}
