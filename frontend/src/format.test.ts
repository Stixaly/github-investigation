import { describe, expect, it } from 'vitest';
import { formatDate, initials, roleLabel, statusLabel } from './format';

describe('format', () => {
  it('says when a date is unknown', () => {
    expect(formatDate(null)).toBe('Date inconnue');
    expect(formatDate(undefined)).toBe('Date inconnue');
    expect(formatDate('2026-10-01T00:00:00Z')).toMatch(/2026/);
  });

  it('labels every scan status and role in French', () => {
    expect(statusLabel.waiting_quota).toBe('Attente du quota');
    expect(roleLabel.owner).toBe('Responsable');
    expect(initials('demo-alice')).toBe('DE');
  });
});
