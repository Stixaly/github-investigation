import { useEffect, useState } from 'react';
import { api } from '../api';
import type { GraphData, Member, Note, Scan } from '../types';

const SCAN_REFRESH_MS = 5000;

/** Scans of an investigation, refreshed while the page is open. */
export function useScans(investigationId: string, onError: (message: string) => void) {
  const [scans, setScans] = useState<Scan[]>([]);
  useEffect(() => {
    let live = true;
    const update = async () => {
      try {
        const rows = await api<Scan[]>(`/investigations/${investigationId}/scans`);
        if (live) setScans(rows);
      } catch (failure) {
        if (live) onError((failure as Error).message);
      }
    };
    void update();
    const timer = setInterval(update, SCAN_REFRESH_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [investigationId, onError]);
  return [scans, setScans] as const;
}

/** Graph of a scan, reloaded when the scan changes state (a running scan fills in). */
export function useScanGraph(scanId: string, status: string | undefined, onError: (message: string) => void) {
  const [graph, setGraph] = useState<GraphData | null>(null);
  useEffect(() => {
    if (!scanId) return;
    let live = true;
    api<GraphData>(`/scans/${scanId}/graph`)
      .then((data) => live && setGraph(data))
      .catch((failure: Error) => live && onError(failure.message));
    return () => {
      live = false;
    };
  }, [scanId, status, onError]);
  return [scanId ? graph : null, setGraph] as const;
}

/** Notes and members, reloaded whenever the analyst switches tab. */
export function useNotesAndMembers(investigationId: string, tab: string, onError: (message: string) => void) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [members, setMembers] = useState<Member[]>([]);
  useEffect(() => {
    let live = true;
    Promise.all([
      api<Note[]>(`/investigations/${investigationId}/notes`),
      api<Member[]>(`/investigations/${investigationId}/members`),
    ])
      .then(([noteRows, memberRows]) => {
        if (live) {
          setNotes(noteRows);
          setMembers(memberRows);
        }
      })
      .catch((failure: Error) => live && onError(failure.message));
    return () => {
      live = false;
    };
  }, [investigationId, tab, onError]);
  return { notes, setNotes, members, setMembers };
}
