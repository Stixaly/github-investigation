import { useEffect, useMemo, useRef, useState } from 'react';
import type { Core } from 'cytoscape';
import {
  Download,
  Expand,
  Focus,
  Maximize2,
  Minimize2,
  Minus,
  Network,
  PanelRight,
  Plus,
  RotateCcw,
  Search,
  Tags,
  X,
} from 'lucide-react';
import type { Edge, GraphData, Node } from '../types';
import type { Engine3D } from './force3d';
import {
  arrange,
  colorOf,
  cytoscapeStyle,
  icon,
  kindOf,
  kinds,
  relationLabel,
  search,
  type Kind,
} from './model';
import './graph.css';

type Props = {
  data: GraphData;
  onSelect: (item: Node | Edge) => void;
  selectedId?: string;
};

/** Graph workbench: 3D force view by default, Cytoscape 2D view as an alternative and fallback. */
export default function Graph({ data, onSelect, selectedId }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const core = useRef<Core | null>(null);
  const force = useRef<Engine3D | null>(null);
  const [mode, setMode] = useState<'3d' | '2d'>('3d');
  const [ready, setReady] = useState(false);
  const [rendererNotice, setRendererNotice] = useState('');
  const callback = useRef(onSelect);
  callback.current = onSelect;
  const [layout, setLayout] = useState('cose');
  const [labels, setLabels] = useState(true);
  const [hidden, setHidden] = useState<Kind[]>([]);
  const [isolate, setIsolate] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [inspector, setInspector] = useState(true);
  const [query, setQuery] = useState('');
  const [zoom, setZoom] = useState(100);
  const [visible, setVisible] = useState({ nodes: 0, edges: 0 });
  const [exportError, setExportError] = useState('');
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const selectedNode = data.nodes.find((node) => node.id === selectedId);
  const neighbors = useMemo(() => {
    const ids = new Set<string>();
    if (selectedId) ids.add(selectedId);
    for (const edge of data.edges) {
      if (edge.id === selectedId || edge.source_id === selectedId || edge.target_id === selectedId) {
        ids.add(edge.source_id);
        ids.add(edge.target_id);
      }
    }
    return ids;
  }, [data, selectedId]);
  const visibleIds = useMemo(
    () =>
      new Set(
        data.nodes
          .filter(
            (node) => !hidden.includes(kindOf(node)) && (!isolate || !selectedNode || neighbors.has(node.id)),
          )
          .map((node) => node.id),
      ),
    [data, hidden, isolate, selectedNode, neighbors],
  );
  const matches = useMemo(() => search(data.nodes, query, hidden), [data, query, hidden]);

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    setReady(false);
    let cancelled = false;
    if (mode === '3d') {
      const fallback = () => {
        if (!cancelled) {
          setRendererNotice('Le rendu 3D est indisponible dans ce navigateur. La vue 2D est activée.');
          setMode('2d');
        }
      };
      import('./force3d')
        .then(({ createForce3D }) => {
          if (cancelled) return;
          force.current = createForce3D(element, data, {
            color: colorOf,
            relation: relationLabel,
            onSelect: (item) => callback.current(item),
            onZoom: setZoom,
            onLost: fallback,
          });
          setReady(true);
        })
        .catch(fallback);
      return () => {
        cancelled = true;
        force.current?.destroy();
        force.current = null;
      };
    }
    let observer: ResizeObserver | undefined;
    // Cytoscape is only downloaded when the 2D view is used.
    import('cytoscape')
      .then(({ default: cytoscape }) => {
        if (cancelled) return;
        const cy = cytoscape({
          container: element,
          minZoom: 0.08,
          maxZoom: 4,
          elements: [
            ...data.nodes.map((node) => ({
              data: {
                id: node.id,
                label: node.label,
                kind: kindOf(node),
                icon: icon(kindOf(node)),
                color: colorOf(node),
              },
            })),
            ...data.edges.map((edge) => ({
              data: {
                id: edge.id,
                source: edge.source_id,
                target: edge.target_id,
                label: relationLabel(edge.kind),
              },
            })),
          ],
          layout: { name: 'preset' },
          style: cytoscapeStyle,
        });
        core.current = cy;
        cy.on('tap', 'node, edge', (event) => {
          const id = event.target.id();
          const item = data.nodes.find((node) => node.id === id) ?? data.edges.find((edge) => edge.id === id);
          if (item) callback.current(item);
        });
        cy.on('zoom', () => setZoom(Math.round(cy.zoom() * 100)));
        arrange(cy, layoutRef.current);
        observer = new ResizeObserver(() => {
          cy.resize();
          cy.fit(cy.elements(':visible'), 65);
        });
        observer.observe(element);
        setReady(true);
      })
      .catch(() => {
        if (!cancelled) setRendererNotice('La vue 2D n’a pas pu être chargée. Recharger la page.');
      });
    return () => {
      cancelled = true;
      observer?.disconnect();
      core.current?.destroy();
      core.current = null;
    };
  }, [data, mode]);

  useEffect(() => {
    if (mode === '3d') {
      force.current?.update({ ids: visibleIds, selected: selectedId, neighbors, labels });
      setVisible({
        nodes: visibleIds.size,
        edges: data.edges.filter((edge) => visibleIds.has(edge.source_id) && visibleIds.has(edge.target_id))
          .length,
      });
      return;
    }
    const cy = core.current;
    if (!cy) return;
    const active = selectedId ? cy.getElementById(selectedId) : cy.collection();
    const neighborhood = selectedNode ? active.closedNeighborhood() : active.union(active.connectedNodes());
    cy.batch(() => {
      cy.elements().removeClass('hidden muted').unselect();
      cy.nodes().forEach((node) => {
        if (hidden.includes(node.data('kind')) || (isolate && selectedNode && !neighborhood.contains(node)))
          node.addClass('hidden');
      });
      cy.edges().forEach((edge) => {
        if (edge.source().hasClass('hidden') || edge.target().hasClass('hidden')) edge.addClass('hidden');
      });
      if (active.length) {
        active.select();
        cy.elements().difference(neighborhood).addClass('muted');
      }
      cy.edges().toggleClass('labels-off', !labels);
    });
    // Read the filter classes directly; Cytoscape updates computed visibility after batching.
    setVisible({
      nodes: cy.nodes().filter((node) => !node.hasClass('hidden')).length,
      edges: cy.edges().filter((edge) => !edge.hasClass('hidden')).length,
    });
  }, [data, hidden, isolate, labels, selectedId, selectedNode, mode, ready, visibleIds, neighbors]);

  useEffect(() => {
    const explorer = container.current?.closest('.explorer');
    explorer?.classList.toggle('graph-expanded', expanded);
    const previousOverflow = document.body.style.overflow;
    if (expanded) document.body.style.overflow = 'hidden';
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setExpanded(false);
    };
    document.addEventListener('keydown', escape);
    return () => {
      explorer?.classList.remove('graph-expanded');
      document.body.style.overflow = previousOverflow;
      document.removeEventListener('keydown', escape);
    };
  }, [expanded]);

  useEffect(() => {
    const explorer = container.current?.closest('.explorer');
    explorer?.classList.toggle('inspector-hidden', !inspector);
    return () => explorer?.classList.remove('inspector-hidden');
  }, [inspector]);

  const fit = () => {
    if (mode === '3d') {
      force.current?.fit();
      return;
    }
    const cy = core.current;
    if (cy) cy.fit(cy.elements(':visible'), 65);
  };
  const changeZoom = (factor: number) => {
    if (mode === '3d') {
      force.current?.zoom(factor);
      return;
    }
    const cy = core.current;
    if (cy)
      cy.zoom({
        level: Math.min(4, Math.max(0.08, cy.zoom() * factor)),
        renderedPosition: { x: cy.width() / 2, y: cy.height() / 2 },
      });
  };
  const changeLayout = (value: string) => {
    setLayout(value);
    if (mode === '3d') force.current?.arrange(value);
    else if (core.current) arrange(core.current, value);
  };
  const find = (node: Node) => {
    setIsolate(false);
    setQuery('');
    callback.current(node);
    if (mode === '3d') {
      force.current?.focus(node.id);
      return;
    }
    const cy = core.current;
    if (cy) {
      cy.zoom(Math.max(0.9, cy.zoom()));
      cy.center(cy.getElementById(node.id));
    }
  };
  const download = () => {
    if (!ready) return;
    try {
      const png =
        mode === '3d' ? force.current?.png() : core.current?.png({ full: false, scale: 2, bg: '#fbfcfa' });
      if (!png) throw new Error('no renderer');
      const link = document.createElement('a');
      link.download = 'investigation-graphe.png';
      link.href = png;
      link.click();
      setExportError('');
    } catch {
      setExportError("L'export n'a pas pu être généré. Réduisez la vue puis réessayez.");
    }
  };

  return (
    <div className="graph-workbench">
      <div className="graph-tools" role="toolbar" aria-label="Outils du graphe">
        <span className="workbench-title">
          <Network size={16} /> ATELIER GRAPHE
        </span>
        <div className="engine-switch" role="group" aria-label="Moteur du graphe">
          <button
            aria-pressed={mode === '3d'}
            onClick={() => {
              if (mode !== '3d') {
                setRendererNotice('');
                setMode('3d');
                setLayout('cose');
              }
            }}
          >
            3D
          </button>
          <button aria-pressed={mode === '2d'} onClick={() => setMode('2d')}>
            2D
          </button>
        </div>
        <label className="layout-choice">
          <span className="sr-only">Disposition du graphe</span>
          <select
            aria-label="Disposition du graphe"
            value={layout}
            disabled={!ready}
            onChange={(event) => changeLayout(event.target.value)}
          >
            <option value="cose">Réseau à forces</option>
            <option value="circle">Circulaire</option>
            <option value="breadthfirst">{mode === '3d' ? 'Par type' : 'Hiérarchique'}</option>
            <option value="grid">{mode === '3d' ? 'Grille 3D' : 'Grille'}</option>
          </select>
        </label>
        <button
          title="Réorganiser les nœuds"
          aria-label="Réorganiser les nœuds"
          disabled={!ready}
          onClick={() => changeLayout(layout)}
        >
          <RotateCcw size={16} />
        </button>
        <span className="tool-divider" />
        <button
          title="Libellés des relations"
          aria-label="Libellés des relations"
          aria-pressed={labels}
          onClick={() => setLabels(!labels)}
        >
          <Tags size={16} />
        </button>
        <button
          title="Isoler le voisinage sélectionné"
          aria-label="Isoler le voisinage sélectionné"
          aria-pressed={isolate}
          disabled={!selectedNode}
          onClick={() => setIsolate(!isolate)}
        >
          <Focus size={17} />
        </button>
        <button
          title="Panneau des preuves"
          aria-label="Panneau des preuves"
          aria-pressed={inspector}
          onClick={() => setInspector(!inspector)}
        >
          <PanelRight size={16} />
        </button>
        <button title="Exporter la vue en PNG" aria-label="Exporter la vue en PNG" onClick={download}>
          <Download size={16} />
        </button>
        <button
          className="expand-graph"
          title={expanded ? 'Réduire le graphe' : 'Agrandir le graphe'}
          aria-label={expanded ? 'Réduire le graphe' : 'Agrandir le graphe'}
          onClick={() => setExpanded(!expanded)}
        >
          {expanded ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
        </button>
      </div>
      <div className="graph-filters" aria-label="Types d’entités">
        {(Object.keys(kinds) as Kind[]).map((kind) => (
          <button
            key={kind}
            aria-pressed={!hidden.includes(kind)}
            onClick={() =>
              setHidden((current) =>
                current.includes(kind) ? current.filter((other) => other !== kind) : [...current, kind],
              )
            }
          >
            <i style={{ background: kinds[kind].color }} />
            {kinds[kind].label}
            <span>{data.nodes.filter((node) => kindOf(node) === kind).length}</span>
          </button>
        ))}
      </div>
      <div className="graph-canvas-wrap">
        <div
          ref={container}
          className="graph"
          data-engine={mode}
          data-ready={ready}
          aria-label="Graphe interactif. Une liste accessible des entités est disponible sous le graphe."
        />
        {!ready && (
          <div className="graph-loading" role="status">
            Préparation du graphe {mode.toUpperCase()}…
          </div>
        )}
        <div className="graph-search">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (matches[0]) find(matches[0]);
            }}
          >
            <Search size={15} />
            <input
              aria-label="Rechercher dans le graphe"
              placeholder="Rechercher une entité…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            {query && (
              <button type="button" aria-label="Effacer la recherche" onClick={() => setQuery('')}>
                <X size={14} />
              </button>
            )}
          </form>
          {query.trim() && (
            <div className="graph-search-results">
              {matches.map((node) => (
                <button key={node.id} onClick={() => find(node)}>
                  <i style={{ background: colorOf(node) }} />
                  <span>{node.label}</span>
                </button>
              ))}
              {!matches.length && <p>Aucune entité visible correspondante.</p>}
            </div>
          )}
        </div>
        {visible.nodes === 0 && (
          <div className="graph-filter-empty">
            Aucune entité affichée.
            <button
              onClick={() => {
                setHidden([]);
                setIsolate(false);
              }}
            >
              Réinitialiser les filtres
            </button>
          </div>
        )}
        <div className="graph-zoom">
          <button aria-label="Zoom arrière" title="Zoom arrière" onClick={() => changeZoom(1 / 1.25)}>
            <Minus size={16} />
          </button>
          <output aria-label="Niveau de zoom">{zoom}%</output>
          <button aria-label="Zoom avant" title="Zoom avant" onClick={() => changeZoom(1.25)}>
            <Plus size={16} />
          </button>
          <span />
          <button aria-label="Cadrer le graphe" title="Cadrer le graphe" onClick={fit}>
            <Expand size={16} />
          </button>
        </div>
        <div className="graph-stats" role="status" data-nodes={visible.nodes} data-edges={visible.edges}>
          {visible.nodes} entités · {visible.edges} relations{isolate && selectedNode ? ' · voisinage' : ''}
        </div>
        <span className="graph-hint">
          {mode === '3d'
            ? 'Glisser pour tourner · Clic droit pour déplacer · Molette pour zoomer'
            : 'Glisser pour déplacer · Molette pour zoomer'}
        </span>
      </div>
      {exportError && (
        <p className="error" role="alert">
          {exportError}
        </p>
      )}
      {rendererNotice && (
        <p className="graph-renderer-notice" role="status">
          {rendererNotice}
        </p>
      )}
    </div>
  );
}
