import type { Core, LayoutOptions, StylesheetJson } from 'cytoscape';
import type { Node } from '../types';

export const kinds = {
  Account: {
    label: 'Comptes',
    color: '#188978',
    path: '<circle cx="12" cy="8" r="4"/><path d="M4 22v-3a8 8 0 0 1 16 0v3"/>',
  },
  Organization: {
    label: 'Organisations',
    color: '#7c8860',
    path: '<path d="M4 22V3h16v19M2 22h20M8 7h2m4 0h2M8 11h2m4 0h2M9 22v-6h6v6"/>',
  },
  Repository: {
    label: 'Dépôts',
    color: '#5d68ba',
    path: '<path d="M6 3h14v18H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3Zm-3 15a3 3 0 0 1 3-3h14M8 6v6m4-6h4"/>',
  },
  Issue: {
    label: 'Issues',
    color: '#c58a35',
    path: '<circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 3v1"/>',
  },
  PullRequest: {
    label: 'Pull requests',
    color: '#b76891',
    path: '<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="19" r="2"/><path d="M6 7v10m12 0V9a4 4 0 0 0-4-4h-2m3-3-3 3 3 3"/>',
  },
};

export type Kind = keyof typeof kinds;

/** Display kind of a node: organizations are accounts whose GitHub type says so. */
export function kindOf(node: Node): Kind {
  if (node.kind === 'Account' && node.data.type === 'Organization') return 'Organization';
  return node.kind in kinds ? (node.kind as Kind) : 'Account';
}

export function colorOf(node: Node): string {
  return kinds[kindOf(node)].color;
}

const relationLabels: Record<string, string> = {
  OWNS: 'possède',
  FORK_OF: 'fork de',
  CONTRIBUTED_TO: 'contribue à',
  AUTHORED: 'auteur de',
  IN_REPOSITORY: 'dans le dépôt',
  STARRED: 'a ajouté une étoile',
  LEGACY_ROLE: 'relation historique',
};

export function relationLabel(kind: string): string {
  return relationLabels[kind] ?? kind;
}

/** SVG data URI of a kind's icon, drawn in its color. */
export function icon(kind: Kind): string {
  const meta = kinds[kind];
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" ` +
    `stroke="${meta.color}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${meta.path}</svg>`;
  return 'data:image/svg+xml;utf8,' + encodeURIComponent(svg);
}

/** Entities whose label contains the query, among the visible kinds. */
export function search(nodes: Node[], query: string, hidden: Kind[], max = 8): Node[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  return nodes
    .filter((node) => !hidden.includes(kindOf(node)) && node.label.toLowerCase().includes(needle))
    .slice(0, max);
}

export function arrange(cy: Core, name: string) {
  const elements = cy.elements(':visible');
  if (!elements.nodes().length) return;
  const options =
    name === 'cose'
      ? {
          name,
          randomize: false,
          nodeRepulsion: 14000,
          idealEdgeLength: 130,
          gravity: 0.18,
          numIter: 450,
          componentSpacing: 130,
          nodeOverlap: 30,
        }
      : { name, avoidOverlap: true, spacingFactor: 1.6, directed: true };
  elements.layout({ ...options, animate: false, padding: 65, fit: true } as LayoutOptions).run();
}

export const cytoscapeStyle: StylesheetJson = [
  {
    selector: 'node',
    style: {
      width: 42,
      height: 42,
      'background-color': '#ffffff',
      'background-image': 'data(icon)',
      'background-width': 27,
      'background-height': 27,
      'border-width': 1.5,
      'border-color': 'data(color)',
      label: 'data(label)',
      color: '#28483f',
      'font-size': 12,
      'font-family': 'system-ui, sans-serif',
      'text-valign': 'bottom',
      'text-margin-y': 10,
      'text-max-width': '165px',
      'text-wrap': 'ellipsis',
      'text-background-color': '#fbfcfa',
      'text-background-opacity': 0.9,
      'text-background-padding': '3px',
      'min-zoomed-font-size': 6,
    },
  },
  { selector: 'node[kind="Repository"]', style: { shape: 'round-rectangle' } },
  { selector: 'node[kind="Organization"]', style: { shape: 'round-hexagon' } },
  {
    selector: 'edge',
    style: {
      width: 1.2,
      'line-color': '#91a99f',
      'target-arrow-color': '#708e83',
      'target-arrow-shape': 'triangle',
      'curve-style': 'bezier',
      'arrow-scale': 0.8,
      label: 'data(label)',
      color: '#59776c',
      'font-size': 10,
      'text-rotation': 'autorotate',
      'text-background-color': '#fbfcfa',
      'text-background-opacity': 0.92,
      'text-background-padding': '3px',
      'min-zoomed-font-size': 7,
    },
  },
  { selector: '.muted', style: { opacity: 0.17 } },
  { selector: '.hidden', style: { display: 'none' } },
  { selector: 'edge.labels-off', style: { label: '' } },
  {
    selector: 'node:selected',
    style: {
      'border-width': 3,
      'border-color': '#176b5b',
      'background-color': '#e2f0e9',
      'text-outline-color': '#fbfcfa',
      'text-outline-width': 2,
      'z-index': 10,
    },
  },
  {
    selector: 'edge:selected',
    style: {
      width: 3,
      'line-color': '#176b5b',
      'target-arrow-color': '#176b5b',
      color: '#176b5b',
      'z-index': 10,
    },
  },
];
