import { useCallback, useState } from 'react';
import { Users } from 'lucide-react';
import { api } from '../api';
import type { Action } from '../hooks/useAction';
import { useGraph, useNotesAndMembers, useScans } from '../hooks/useInvestigationData';
import type { Investigation, Scan } from '../types';
import { ALL_SCANS } from '../types';
import { AccessTab } from './AccessTab';
import { ExplorationTab } from './ExplorationTab';
import { NotesTab } from './NotesTab';
import { TimelineTab } from './TimelineTab';

const TABS = [
  ['graph', 'Exploration'],
  ['timeline', 'Scans & chronologie'],
  ['notes', 'Notes d’analyse'],
  ['access', 'Accès'],
] as const;

type Tab = (typeof TABS)[number][0];

type Props = {
  investigation: Investigation;
  action: Action;
  onInvestigationsChanged: () => Promise<void>;
  onOpenInvestigation: (id: string) => void;
};

/** One investigation; mounted with `key={investigation.id}` so its state resets on switch. */
export function InvestigationView({
  investigation,
  action,
  onInvestigationsChanged,
  onOpenInvestigation,
}: Props) {
  const { setError } = action;
  const [tab, setTab] = useState<Tab>('graph');
  const [chosenScan, setChosenScan] = useState('');
  const [scans, setScans] = useScans(investigation.id, setError);
  const cumulative = chosenScan === ALL_SCANS;
  const scan = cumulative ? undefined : (scans.find((row) => row.id === chosenScan) ?? scans[0]);
  const graphPath = cumulative
    ? `/investigations/${investigation.id}/graph`
    : scan
      ? `/scans/${scan.id}/graph`
      : '';
  // The cumulative view changes whenever any scan does.
  const version = cumulative ? scans.map((row) => row.status).join() : (scan?.status ?? '');
  const [graph, setGraph] = useGraph(graphPath, version, setError);
  const { notes, setNotes, members, setMembers } = useNotesAndMembers(investigation.id, tab, setError);
  const canWrite = investigation.role !== 'reader';

  const reloadScans = useCallback(async () => {
    setScans(await api<Scan[]>(`/investigations/${investigation.id}/scans`));
  }, [investigation.id, setScans]);

  return (
    <>
      <section className="heading">
        <div>
          <p className="eyebrow">DOSSIER D’INVESTIGATION</p>
          <h1>{investigation.title}</h1>
          <p>{investigation.description || 'Explorez les liens, documentez vos observations.'}</p>
        </div>
        <span className="count-label">
          <Users size={16} />
          {members.length} membre{members.length > 1 ? 's' : ''}
        </span>
      </section>
      <div className="tabs">
        {TABS.map(([id, label]) => (
          <button key={id} className={tab === id ? 'selected' : ''} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      {tab === 'graph' && (
        <ExplorationTab
          investigationId={investigation.id}
          canWrite={canWrite}
          scans={scans}
          scan={scan}
          view={cumulative ? ALL_SCANS : (scan?.id ?? '')}
          graphPath={graphPath}
          graph={graph}
          action={action}
          onScanCreated={(created) => {
            setScans((rows) => [created, ...rows.filter((row) => row.id !== created.id)]);
            setChosenScan(created.id);
            void onInvestigationsChanged();
          }}
          onSelectScan={setChosenScan}
          onGraph={setGraph}
          onOpenInvestigation={onOpenInvestigation}
        />
      )}
      {tab === 'timeline' && (
        <TimelineTab
          scans={scans}
          canWrite={canWrite}
          action={action}
          onExplore={(id) => {
            setChosenScan(id);
            setTab('graph');
          }}
          onScansChanged={reloadScans}
        />
      )}
      {tab === 'notes' && (
        <NotesTab
          investigationId={investigation.id}
          notes={notes}
          canWrite={canWrite}
          action={action}
          onNotes={setNotes}
        />
      )}
      {tab === 'access' && (
        <AccessTab
          investigationId={investigation.id}
          members={members}
          isOwner={investigation.role === 'owner'}
          action={action}
          onMembers={setMembers}
        />
      )}
    </>
  );
}
