import type { Role, ScanStatus } from './types';

export function formatDate(value: string | null | undefined): string {
  return value
    ? new Date(value).toLocaleString('fr-FR', { dateStyle: 'medium', timeStyle: 'short' })
    : 'Date inconnue';
}

export const statusLabel: Record<ScanStatus, string> = {
  queued: 'En attente',
  running: 'Collecte en cours',
  waiting_quota: 'Attente du quota',
  succeeded: 'Terminé',
  partial: 'Partiel',
  failed: 'Échec',
  cancelled: 'Annulé',
};

export const roleLabel: Record<Role, string> = {
  owner: 'Responsable',
  analyst: 'Analyste',
  reader: 'Lecteur',
};

export const kindLabel: Record<string, string> = {
  Account: 'Compte',
  Repository: 'Dépôt',
  Issue: 'Issue',
  PullRequest: 'Pull request',
};

export function initials(login: string): string {
  return login.slice(0, 2).toUpperCase();
}
