import {useEffect, useMemo, useRef, useState} from 'react';
import cytoscape, {type Core, type LayoutOptions} from 'cytoscape';
import {Download, Expand, Focus, Maximize2, Minimize2, Minus, Network, PanelRight, Plus, RotateCcw, Search, Tags, X} from 'lucide-react';
import type {Engine3D} from './force3d';

export type Node = {id:string;kind:string;label:string;evidence_hash:string;observed_at:string;time_precision:string;provisional:boolean;data:Record<string,unknown>};
export type Edge = {id:string;source_id:string;target_id:string;kind:string;evidence_hash:string;observed_at:string};
export type GraphData = {nodes:Node[];edges:Edge[];truncated:boolean;notice:string};

const kinds = {
  Account: {label:'Comptes', color:'#188978', path:'<circle cx="12" cy="8" r="4"/><path d="M4 22v-3a8 8 0 0 1 16 0v3"/>'},
  Organization: {label:'Organisations', color:'#7c8860', path:'<path d="M4 22V3h16v19M2 22h20M8 7h2m4 0h2M8 11h2m4 0h2M9 22v-6h6v6"/>'},
  Repository: {label:'Dépôts', color:'#5d68ba', path:'<path d="M6 3h14v18H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3Zm-3 15a3 3 0 0 1 3-3h14M8 6v6m4-6h4"/>'},
  Issue: {label:'Issues', color:'#c58a35', path:'<circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 3v1"/>'},
  PullRequest: {label:'Pull requests', color:'#b76891', path:'<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="19" r="2"/><path d="M6 7v10m12 0V9a4 4 0 0 0-4-4h-2m3-3-3 3 3 3"/>'},
};
type Kind = keyof typeof kinds;
const kindOf = (n:Node):Kind => n.kind==='Account' && n.data.type==='Organization' ? 'Organization' : n.kind as Kind;
const relations:Record<string,string> = {OWNS:'possède', FORK_OF:'fork de', CONTRIBUTED_TO:'contribue à', AUTHORED:'auteur de', IN_REPOSITORY:'dans le dépôt', STARRED:'a ajouté une étoile', LEGACY_ROLE:'relation historique'};
const icon = (kind:Kind) => {
  const meta=kinds[kind] || kinds.Account;
  return 'data:image/svg+xml;utf8,'+encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="${meta.color}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${meta.path}</svg>`);
};

function arrange(cy:Core, name:string) {
  const elements=cy.elements(':visible');
  if(!elements.nodes().length)return;
  const options = name==='cose'
    ? {name, randomize:false, nodeRepulsion:14000, idealEdgeLength:130, gravity:0.18, numIter:450, componentSpacing:130, nodeOverlap:30}
    : {name, avoidOverlap:true, spacingFactor:1.6, directed:true};
  elements.layout({...options, animate:false, padding:65, fit:true} as LayoutOptions).run();
}

export default function Graph({data, onSelect, selectedId}:{data:GraphData;onSelect:(item:Node|Edge)=>void;selectedId?:string}) {
  const container=useRef<HTMLDivElement>(null), core=useRef<Core|null>(null);
  const force=useRef<Engine3D|null>(null);
  const [mode,setMode]=useState<'3d'|'2d'>('3d'),[ready,setReady]=useState(false),[rendererNotice,setRendererNotice]=useState('');
  const callback=useRef(onSelect);
  callback.current=onSelect;
  const [layout,setLayout]=useState('cose'), [labels,setLabels]=useState(true);
  const [hidden,setHidden]=useState<Kind[]>([]), [isolate,setIsolate]=useState(false);
  const [expanded,setExpanded]=useState(false), [inspector,setInspector]=useState(true);
  const [query,setQuery]=useState(''), [zoom,setZoom]=useState(100), [visible,setVisible]=useState({nodes:0,edges:0});
  const [exportError,setExportError]=useState('');
  const layoutRef=useRef(layout);
  layoutRef.current=layout;
  const selectedNode=data.nodes.find(n=>n.id===selectedId);
  const neighbors=useMemo(()=>{
    const ids=new Set<string>();
    if(selectedId)ids.add(selectedId);
    data.edges.forEach(e=>{if(e.id===selectedId||e.source_id===selectedId||e.target_id===selectedId){ids.add(e.source_id);ids.add(e.target_id)}});
    return ids;
  },[data,selectedId]);
  const visibleIds=useMemo(()=>new Set(data.nodes.filter(n=>!hidden.includes(kindOf(n))&&(!isolate||!selectedNode||neighbors.has(n.id))).map(n=>n.id)),[data,hidden,isolate,selectedNode,neighbors]);
  const matches=useMemo(()=>query.trim() ? data.nodes.filter(n=>!hidden.includes(kindOf(n)) && n.label.toLowerCase().includes(query.trim().toLowerCase())).slice(0,8) : [],[data,query,hidden]);

  useEffect(()=>{
    if(!container.current)return;
    setReady(false);
    if(mode==='3d'){
      let cancelled=false;
      const element=container.current;
      const fallback=()=>{if(!cancelled){setRendererNotice('Le rendu 3D est indisponible dans ce navigateur. La vue 2D est activée.');setMode('2d')}};
      import('./force3d').then(({createForce3D})=>{
        if(cancelled)return;
        force.current=createForce3D(element,data,{color:n=>(kinds[kindOf(n)]||kinds.Account).color,relation:kind=>relations[kind]||kind,onSelect:item=>callback.current(item),onZoom:setZoom,onLost:fallback});
        setReady(true);
      }).catch(fallback);
      return()=>{cancelled=true;force.current?.destroy();force.current=null};
    }
    const cy=cytoscape({container:container.current, minZoom:0.08, maxZoom:4,
      elements:[...data.nodes.map(n=>({data:{id:n.id,label:n.label,kind:kindOf(n),icon:icon(kindOf(n)),color:(kinds[kindOf(n)]||kinds.Account).color}})),
        ...data.edges.map(e=>({data:{id:e.id,source:e.source_id,target:e.target_id,label:relations[e.kind]||e.kind}}))],
      layout:{name:'preset'},
      style:[
        {selector:'node',style:{width:42,height:42,'background-color':'#ffffff','background-image':'data(icon)','background-width':27,'background-height':27,'border-width':1.5,'border-color':'data(color)',label:'data(label)',color:'#28483f','font-size':12,'font-family':'system-ui, sans-serif','text-valign':'bottom','text-margin-y':10,'text-max-width':'165px','text-wrap':'ellipsis','text-background-color':'#fbfcfa','text-background-opacity':0.9,'text-background-padding':'3px','min-zoomed-font-size':6}},
        {selector:'node[kind="Repository"]',style:{shape:'round-rectangle'}},
        {selector:'node[kind="Organization"]',style:{shape:'round-hexagon'}},
        {selector:'edge',style:{width:1.2,'line-color':'#91a99f','target-arrow-color':'#708e83','target-arrow-shape':'triangle','curve-style':'bezier','arrow-scale':0.8,label:'data(label)',color:'#59776c','font-size':10,'text-rotation':'autorotate','text-background-color':'#fbfcfa','text-background-opacity':0.92,'text-background-padding':'3px','min-zoomed-font-size':7}},
        {selector:'.muted',style:{opacity:0.17}},
        {selector:'.hidden',style:{display:'none'}},
        {selector:'edge.labels-off',style:{label:''}},
        {selector:'node:selected',style:{'border-width':3,'border-color':'#176b5b','background-color':'#e2f0e9','text-outline-color':'#fbfcfa','text-outline-width':2,'z-index':10}},
        {selector:'edge:selected',style:{width:3,'line-color':'#176b5b','target-arrow-color':'#176b5b',color:'#176b5b','z-index':10}},
      ]});
    core.current=cy;
    cy.on('tap','node, edge',e=>{
      const item=data.nodes.find(n=>n.id===e.target.id()) || data.edges.find(edge=>edge.id===e.target.id());
      if(item)callback.current(item);
    });
    cy.on('zoom',()=>setZoom(Math.round(cy.zoom()*100)));
    arrange(cy,layoutRef.current);
    const observer=new ResizeObserver(()=>{cy.resize();cy.fit(cy.elements(':visible'),65)});
    observer.observe(container.current);
    setReady(true);
    return()=>{observer.disconnect();cy.destroy();core.current=null};
  },[data,mode]);

  useEffect(()=>{
    if(mode==='3d'){
      force.current?.update({ids:visibleIds,selected:selectedId,neighbors,labels});
      setVisible({nodes:visibleIds.size,edges:data.edges.filter(e=>visibleIds.has(e.source_id)&&visibleIds.has(e.target_id)).length});
      return;
    }
    const cy=core.current;if(!cy)return;
    const active=selectedId ? cy.getElementById(selectedId) : cy.collection();
    const neighborhood=selectedNode ? active.closedNeighborhood() : active.union(active.connectedNodes());
    cy.batch(()=>{
      cy.elements().removeClass('hidden muted').unselect();
      cy.nodes().forEach(node=>{if(hidden.includes(node.data('kind')) || (isolate && selectedNode && !neighborhood.contains(node)))node.addClass('hidden')});
      cy.edges().forEach(edge=>{if(edge.source().hasClass('hidden') || edge.target().hasClass('hidden'))edge.addClass('hidden')});
      if(active.length){active.select();cy.elements().difference(neighborhood).addClass('muted')}
      cy.edges().toggleClass('labels-off',!labels);
    });
    // Read the filter classes directly; Cytoscape updates computed visibility after batching.
    setVisible({nodes:cy.nodes().filter(n=>!n.hasClass('hidden')).length,edges:cy.edges().filter(e=>!e.hasClass('hidden')).length});
  },[data,hidden,isolate,labels,selectedId,selectedNode,mode,ready,visibleIds,neighbors]);

  useEffect(()=>{
    const explorer=container.current?.closest('.explorer');
    explorer?.classList.toggle('graph-expanded',expanded);
    const oldOverflow=document.body.style.overflow;
    if(expanded)document.body.style.overflow='hidden';
    const escape=(event:KeyboardEvent)=>{if(event.key==='Escape')setExpanded(false)};
    document.addEventListener('keydown',escape);
    return()=>{explorer?.classList.remove('graph-expanded');document.body.style.overflow=oldOverflow;document.removeEventListener('keydown',escape)};
  },[expanded]);

  useEffect(()=>{
    const explorer=container.current?.closest('.explorer');
    explorer?.classList.toggle('inspector-hidden',!inspector);
    return()=>explorer?.classList.remove('inspector-hidden');
  },[inspector]);

  const fit=()=>{if(mode==='3d'){force.current?.fit();return}const cy=core.current;if(cy)cy.fit(cy.elements(':visible'),65)};
  const changeZoom=(factor:number)=>{if(mode==='3d'){force.current?.zoom(factor);return}const cy=core.current;if(cy)cy.zoom({level:Math.min(4,Math.max(0.08,cy.zoom()*factor)),renderedPosition:{x:cy.width()/2,y:cy.height()/2}})};
  const changeLayout=(value:string)=>{setLayout(value);if(mode==='3d')force.current?.arrange(value);else if(core.current)arrange(core.current,value)};
  const find=(node:Node)=>{
    setIsolate(false);setQuery('');callback.current(node);
    if(mode==='3d'){force.current?.focus(node.id);return}
    const cy=core.current;if(cy){cy.zoom(Math.max(0.9,cy.zoom()));cy.center(cy.getElementById(node.id))}
  };
  const download=()=>{
    if(!ready)return;
    try {
      const link=document.createElement('a');
      link.download='investigation-graphe.png';
      link.href=mode==='3d'?force.current!.png():core.current!.png({full:false,scale:2,bg:'#fbfcfa'});link.click();setExportError('');
    } catch {setExportError("L'export n'a pas pu être généré. Réduisez la vue puis réessayez.")}
  };
  return <div className="graph-workbench">
    <div className="graph-tools" role="toolbar" aria-label="Outils du graphe">
      <span className="workbench-title"><Network size={16}/> ATELIER GRAPHE</span>
      <div className="engine-switch" role="group" aria-label="Moteur du graphe"><button aria-pressed={mode==='3d'} onClick={()=>{if(mode!=='3d'){setRendererNotice('');setMode('3d');setLayout('cose')}}}>3D</button><button aria-pressed={mode==='2d'} onClick={()=>setMode('2d')}>2D</button></div>
      <label className="layout-choice"><span className="sr-only">Disposition du graphe</span><select aria-label="Disposition du graphe" value={layout} disabled={!ready} onChange={e=>changeLayout(e.target.value)}><option value="cose">Réseau à forces</option><option value="circle">Circulaire</option><option value="breadthfirst">{mode==='3d'?'Par type':'Hiérarchique'}</option><option value="grid">{mode==='3d'?'Grille 3D':'Grille'}</option></select></label>
      <button title="Réorganiser les nœuds" aria-label="Réorganiser les nœuds" disabled={!ready} onClick={()=>changeLayout(layout)}><RotateCcw size={16}/></button>
      <span className="tool-divider"/>
      <button title="Libellés des relations" aria-label="Libellés des relations" aria-pressed={labels} onClick={()=>setLabels(!labels)}><Tags size={16}/></button>
      <button title="Isoler le voisinage sélectionné" aria-label="Isoler le voisinage sélectionné" aria-pressed={isolate} disabled={!selectedNode} onClick={()=>setIsolate(!isolate)}><Focus size={17}/></button>
      <button title="Panneau des preuves" aria-label="Panneau des preuves" aria-pressed={inspector} onClick={()=>setInspector(!inspector)}><PanelRight size={16}/></button>
      <button title="Exporter la vue en PNG" aria-label="Exporter la vue en PNG" onClick={download}><Download size={16}/></button>
      <button className="expand-graph" title={expanded?'Réduire le graphe':'Agrandir le graphe'} aria-label={expanded?'Réduire le graphe':'Agrandir le graphe'} onClick={()=>setExpanded(!expanded)}>{expanded?<Minimize2 size={16}/>:<Maximize2 size={16}/>}</button>
    </div>
    <div className="graph-filters" aria-label="Types d’entités">
      {(Object.keys(kinds) as Kind[]).map(kind=><button key={kind} aria-pressed={!hidden.includes(kind)} onClick={()=>setHidden(current=>current.includes(kind)?current.filter(k=>k!==kind):[...current,kind])}><i style={{background:kinds[kind].color}}/>{kinds[kind].label}<span>{data.nodes.filter(n=>kindOf(n)===kind).length}</span></button>)}
    </div>
    <div className="graph-canvas-wrap">
      <div ref={container} className="graph" data-engine={mode} data-ready={ready} aria-label="Graphe interactif. Une liste accessible des entités est disponible sous le graphe."/>
      {!ready&&<div className="graph-loading" role="status">Préparation du graphe {mode.toUpperCase()}…</div>}
      <div className="graph-search"><form onSubmit={e=>{e.preventDefault();if(matches[0])find(matches[0])}}><Search size={15}/><input aria-label="Rechercher dans le graphe" placeholder="Rechercher une entité…" value={query} onChange={e=>setQuery(e.target.value)}/>{query&&<button type="button" aria-label="Effacer la recherche" onClick={()=>setQuery('')}><X size={14}/></button>}</form>{query.trim()&&<div className="graph-search-results">{matches.map(n=><button key={n.id} onClick={()=>find(n)}><i style={{background:(kinds[kindOf(n)]||kinds.Account).color}}/><span>{n.label}</span></button>)}{!matches.length&&<p>Aucune entité visible correspondante.</p>}</div>}</div>
      {visible.nodes===0&&<div className="graph-filter-empty">Aucune entité affichée.<button onClick={()=>{setHidden([]);setIsolate(false)}}>Réinitialiser les filtres</button></div>}
      <div className="graph-zoom"><button aria-label="Zoom arrière" title="Zoom arrière" onClick={()=>changeZoom(1/1.25)}><Minus size={16}/></button><output aria-label="Niveau de zoom">{zoom}%</output><button aria-label="Zoom avant" title="Zoom avant" onClick={()=>changeZoom(1.25)}><Plus size={16}/></button><span/><button aria-label="Cadrer le graphe" title="Cadrer le graphe" onClick={fit}><Expand size={16}/></button></div>
      <div className="graph-stats" role="status" data-nodes={visible.nodes} data-edges={visible.edges}>{visible.nodes} entités · {visible.edges} relations{isolate&&selectedNode?' · voisinage':''}</div>
      <span className="graph-hint">{mode==='3d'?'Glisser pour tourner · Clic droit pour déplacer · Molette pour zoomer':'Glisser pour déplacer · Molette pour zoomer'}</span>
    </div>
    {exportError&&<p className="error" role="alert">{exportError}</p>}
    {rendererNotice&&<p className="graph-renderer-notice" role="status">{rendererNotice}</p>}
  </div>;
}
