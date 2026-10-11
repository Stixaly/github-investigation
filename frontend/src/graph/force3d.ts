import ForceGraph3D, { type ForceGraph3DInstance, type NodeObject, type LinkObject } from '3d-force-graph';
import {
  BoxGeometry,
  Group,
  Mesh,
  MeshLambertMaterial,
  OctahedronGeometry,
  SphereGeometry,
  Vector3,
} from 'three';
import SpriteText from 'three-spritetext';
import type { Edge, GraphData, Node } from '../types';

type SimNode = NodeObject & { id: string; record: Node; color: string };
type SimLink = LinkObject<SimNode> & { record: Edge };
type View = { ids: Set<string>; selected?: string; neighbors: Set<string>; labels: boolean };
export type Engine3D = {
  update: (view: View) => void;
  fit: () => void;
  zoom: (factor: number) => void;
  focus: (id: string) => void;
  arrange: (layout: string) => void;
  png: () => string;
  destroy: () => void;
};

const tooltip = (text: string) => {
  const element = document.createElement('span');
  element.textContent = text;
  return element;
};

export function createForce3D(
  container: HTMLDivElement,
  data: GraphData,
  options: {
    color: (node: Node) => string;
    relation: (kind: string) => string;
    onSelect: (item: Node | Edge) => void;
    onZoom: (value: number) => void;
    onLost: () => void;
  },
): Engine3D {
  // Force-graph mutates positions and link endpoints: keep the evidence model immutable.
  const nodes: SimNode[] = data.nodes.map((record) => ({
    id: record.id,
    record,
    color: options.color(record),
  }));
  const links: SimLink[] = data.edges.map((record) => ({
    source: record.source_id,
    target: record.target_id,
    record,
  }));
  let view: View = { ids: new Set(nodes.map((n) => n.id)), neighbors: new Set(), labels: true };
  let initialFit = true,
    alive = true,
    baseline = 500;
  container.dataset.simulation = 'running';
  const nodeObjects = new Map<string, { group: Group; mesh: Mesh; label: SpriteText }>();
  const linkTexts = new Map<string, SpriteText>();
  // The library constructor is not generic; all supplied graph records are constructed above.
  const graph = new ForceGraph3D(container, {
    controlType: 'trackball',
    rendererConfig: { antialias: true, alpha: false, preserveDrawingBuffer: true },
  }) as unknown as ForceGraph3DInstance<SimNode, SimLink>;
  const controls = graph.controls() as {
    target: Vector3;
    addEventListener: (type: string, callback: () => void) => void;
    removeEventListener: (type: string, callback: () => void) => void;
  };
  const distance = () => graph.camera().position.distanceTo(controls.target);
  const reportZoom = () => options.onZoom(Math.round((100 * baseline) / Math.max(1, distance())));
  const fit = () => {
    if (!nodes.some((n) => view.ids.has(n.id))) return;
    graph.zoomToFit(0, 75, (n) => view.ids.has(n.id));
    baseline = Math.max(1, distance());
    reportZoom();
  };
  const refreshObjects = () => {
    nodeObjects.forEach(({ group, mesh, label }, id) => {
      const selected = id === view.selected,
        relevant = !view.selected || view.neighbors.has(id);
      group.scale.setScalar(selected ? 1.3 : 1);
      const material = mesh.material as MeshLambertMaterial;
      material.opacity = relevant ? 1 : 0.25;
      material.emissive.set(selected ? '#174d41' : '#000000');
      label.visible = nodes.length <= 60 || selected;
      label.material.opacity = relevant ? 1 : 0.25;
    });
    linkTexts.forEach((label, id) => {
      label.visible = view.labels && (links.length <= 45 || id === view.selected);
    });
  };
  graph
    .width(container.clientWidth)
    .height(container.clientHeight)
    .backgroundColor('#fbfcfa')
    .showNavInfo(false)
    .nodeLabel((n) => tooltip(n.record.label))
    .linkLabel((l) => tooltip(options.relation(l.record.kind)))
    .nodeThreeObject((node) => {
      const group = new Group();
      const geometry =
        node.record.kind === 'Repository'
          ? new BoxGeometry(6, 6, 6)
          : node.record.kind === 'Issue' || node.record.kind === 'PullRequest'
            ? new OctahedronGeometry(4)
            : new SphereGeometry(3.6, 16, 12);
      const mesh = new Mesh(geometry, new MeshLambertMaterial({ color: node.color, transparent: true }));
      const text = node.record.label.length > 42 ? node.record.label.slice(0, 39) + '…' : node.record.label;
      const label = new SpriteText(text, 4.5, '#28483f');
      // Labels face the camera and can overlap other entities; pick the actual geometry.
      label.raycast = () => {};
      label.backgroundColor = '#fbfcfae6';
      label.padding = 1;
      label.position.y = -7;
      group.add(mesh, label);
      nodeObjects.set(node.id, { group, mesh, label });
      refreshObjects();
      return group;
    })
    .linkColor((link) =>
      view.selected === link.record.id
        ? '#176b5b'
        : view.selected && !view.neighbors.has(link.record.source_id)
          ? '#cfdad5'
          : '#8ba69c',
    )
    .linkWidth((link) => (view.selected === link.record.id ? 1.2 : 0.4))
    .linkOpacity(0.85)
    .linkDirectionalArrowLength(2.5)
    .linkDirectionalArrowRelPos(0.88)
    .linkHoverPrecision(5)
    .linkThreeObjectExtend(true)
    .linkThreeObject((link) => {
      const label = new SpriteText(options.relation(link.record.kind), 3, '#59776c');
      label.raycast = () => {};
      label.backgroundColor = '#fbfcfae6';
      label.padding = 0.5;
      linkTexts.set(link.record.id, label);
      refreshObjects();
      return label;
    })
    .linkPositionUpdate((label, { start, end }) => {
      label.position.set((start.x + end.x) / 2, (start.y + end.y) / 2, (start.z + end.z) / 2);
      return true;
    })
    .warmupTicks(70)
    .cooldownTicks(100)
    .onNodeClick((node) => {
      initialFit = false;
      options.onSelect(node.record);
    })
    .onLinkClick((link) => {
      initialFit = false;
      options.onSelect(link.record);
    })
    .onNodeDragEnd(() => {
      initialFit = false;
    })
    .onEngineStop(() => {
      if (!alive) return;
      container.dataset.simulation = 'idle';
      if (initialFit) {
        initialFit = false;
        fit();
      }
    })
    .graphData({ nodes, links });
  graph.d3Force('charge')?.strength(-110);
  graph.d3Force('link')?.distance(45);
  controls.addEventListener('change', reportZoom);
  const canvas = graph.renderer().domElement;
  const lost = (event: Event) => {
    event.preventDefault();
    if (alive) options.onLost();
  };
  canvas.addEventListener('webglcontextlost', lost);
  const observer = new ResizeObserver(() => {
    if (!alive) return;
    graph.width(container.clientWidth).height(container.clientHeight);
  });
  observer.observe(container);
  const frame = requestAnimationFrame(() => {
    if (alive) fit();
  });

  return {
    update(next) {
      view = next;
      graph
        .nodeVisibility((n) => view.ids.has(n.id))
        .linkVisibility((l) => view.ids.has(l.record.source_id) && view.ids.has(l.record.target_id))
        .linkColor((l) =>
          l.record.id === view.selected
            ? '#176b5b'
            : view.selected && !view.neighbors.has(l.record.source_id)
              ? '#dbe4de'
              : '#8ba69c',
        )
        .linkWidth((l) => (l.record.id === view.selected ? 1.2 : 0.4));
      refreshObjects();
    },
    fit,
    zoom(factor) {
      initialFit = false;
      const position = graph
        .camera()
        .position.clone()
        .sub(controls.target)
        .multiplyScalar(1 / factor)
        .add(controls.target);
      graph.cameraPosition(position, controls.target, 0);
      reportZoom();
    },
    focus(id) {
      const node = nodes.find((n) => n.id === id);
      if (!node) return;
      initialFit = false;
      const target = { x: node.x || 0, y: node.y || 0, z: node.z || 0 };
      graph.cameraPosition({ x: target.x + 55, y: target.y + 25, z: target.z + 100 }, target, 0);
      reportZoom();
    },
    arrange(layout) {
      const visible = nodes.filter((n) => view.ids.has(n.id));
      const groups = ['Account', 'Repository', 'Issue', 'PullRequest'];
      visible.forEach((n, i) => {
        if (layout === 'cose') {
          delete n.fx;
          delete n.fy;
          delete n.fz;
          return;
        }
        if (layout === 'circle') {
          const angle = (2 * Math.PI * i) / Math.max(1, visible.length),
            radius = Math.max(50, visible.length * 6);
          n.fx = Math.cos(angle) * radius;
          n.fy = Math.sin(angle) * radius;
          n.fz = 0;
        } else if (layout === 'grid') {
          const side = Math.ceil(Math.cbrt(visible.length));
          n.fx = (i % side) * 45;
          n.fy = (Math.floor(i / side) % side) * 45;
          n.fz = Math.floor(i / (side * side)) * 45;
        } else {
          const same = visible.filter((v) => v.record.kind === n.record.kind),
            index = same.indexOf(n);
          n.fx = (index % 8) * 40;
          n.fy = groups.indexOf(n.record.kind) * 90;
          n.fz = Math.floor(index / 8) * 40;
        }
      });
      initialFit = true;
      container.dataset.simulation = 'running';
      graph.d3ReheatSimulation();
    },
    png() {
      graph.renderer().render(graph.scene(), graph.camera());
      return canvas.toDataURL('image/png');
    },
    destroy() {
      alive = false;
      cancelAnimationFrame(frame);
      observer.disconnect();
      canvas.removeEventListener('webglcontextlost', lost);
      controls.removeEventListener('change', reportZoom);
      graph._destructor();
      graph.renderer().dispose();
      graph.renderer().forceContextLoss();
      nodeObjects.clear();
      linkTexts.clear();
      container.replaceChildren();
    },
  };
}
