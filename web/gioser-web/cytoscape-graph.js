/**
 * gioser-graph.js — Grafo semántico con Cytoscape.js
 *
 * Se monta automáticamente cuando hay un contenedor <gioser-graph>
 * en el DOM con atributo data-api-url.
 *
 * Efecto "wineandcheesemap": clic en nodo → centra + desvanece resto.
 * Doble clic → callback de navegación.
 */

(function () {
  'use strict';

  const COLORS = {
    logos:  '#d0dbff',
    aire:   '#d0dbff',
    nomos:  '#f59056',
    fuego:  '#f59056',
    kay:    '#d49873',
    tierra: '#d49873',
    uku:    '#6cd0f3',
    agua:   '#6cd0f3',
  };

  function caminoColor(c) { return COLORS[c] || '#888'; }

  function initGraph(container) {
    const apiUrl = container.getAttribute('data-api-url') || 'https://api.gioser.net';
    const onNavigate = window.__gioserGraphNavigate || null;

    fetch(`${apiUrl}/graph?limit=500`)
      .then(r => r.json())
      .then(data => {
        if (!data.nodes || data.nodes.length === 0) return;

        // Construir elementos Cytoscape
        const elements = [];

        for (const n of data.nodes) {
          const d = n.data;
          if (!d.doc_id) continue;
          const color = caminoColor(d.camino);
          elements.push({
            data: {
              id: d.id,
              doc_id: d.doc_id,
              label: d.name.length > 22 ? d.name.slice(0, 20) + '…' : d.name,
              camino: d.camino.toUpperCase(),
              color,
            },
          });
        }

        const nodeIds = new Set(elements.map(e => e.data.id));

        for (const e of data.edges) {
          const ed = e.data;
          if (!nodeIds.has(ed.source) || !nodeIds.has(ed.target)) continue;
          const weight = ed.weight || 0.7;
          elements.push({
            data: {
              id: ed.id,
              source: ed.source,
              target: ed.target,
              weight,
            },
          });
        }

        const cy = cytoscape({
          container,
          elements,
          style: [
            // Aristas: grosor según peso
            {
              selector: 'edge',
              style: {
                'width': 'mapData(weight, 0.5, 1.0, 0.5, 4)',
                'line-color': 'rgba(255,255,255,0.18)',
                'target-arrow-color': 'rgba(255,255,255,0.12)',
                'curve-style': 'haystack',
                'haystack-radius': 0,
                'opacity': 0.6,
              },
            },
            // Nodo: rectángulo redondeado
            {
              selector: 'node',
              style: {
                'shape': 'round-rectangle',
                'width': 130,
                'height': 32,
                'background-color': 'data(color)',
                'background-opacity': 0.20,
                'border-color': 'data(color)',
                'border-width': 1.5,
                'border-opacity': 0.7,
                'color': 'rgba(232,234,245,0.90)',
                'font-size': 11,
                'font-family': 'Inter, system-ui, sans-serif',
                'font-weight': 500,
                'text-valign': 'center',
                'text-halign': 'center',
                'label': 'data(label)',
                'min-zoomed-font-size': 8,
                'shadow-blur': 6,
                'shadow-color': 'rgba(0,0,0,0.4)',
                'shadow-offset-x': 0,
                'shadow-offset-y': 2,
                'shadow-opacity': 0.5,
                'transition-property': 'background-opacity, border-opacity, shadow-blur',
                'transition-duration': 200,
              },
            },
            // Sublabel del camino — lo ponemos como label secundario
            // Cytoscape no soporta dos labels nativamente; usamos un
            // badge de esquina con la data (camino) en el tooltip.
          ],
          layout: {
            name: 'cose',
            animate: false,
            idealEdgeLength: 160,
            nodeRepulsion: 8000,
            gravity: 0.25,
            numIter: 1000,
            fit: true,
            padding: 30,
          },
        });

        // Tooltip con preview al hover
        const tips = {};
        for (const n of data.nodes) {
          const d = n.data;
          if (d.doc_id) tips[d.id] = d;
        }

        let tooltipEl = container.querySelector('.cy-tooltip');
        if (!tooltipEl) {
          tooltipEl = document.createElement('div');
          tooltipEl.className = 'cy-tooltip';
          tooltipEl.style.cssText =
            'position:absolute;z-index:10;pointer-events:none;' +
            'background:rgba(6,5,13,0.88);color:#e8eaf5;' +
            'padding:6px 10px;border-radius:8px;font-size:11px;' +
            'font-family:Inter,sans-serif;line-height:1.4;' +
            'border:1px solid rgba(255,255,255,0.10);' +
            'backdrop-filter:blur(8px);max-width:240px;' +
            'opacity:0;transition:opacity 180ms ease;';
          container.style.position = 'relative';
          container.appendChild(tooltipEl);
        }

        cy.on('mouseover', 'node', (ev) => {
          const node = ev.target;
          node.style({ 'background-opacity': 0.45, 'border-opacity': 1, 'shadow-blur': 12 });
          const tipData = tips[node.id()];
          if (tipData && tipData.preview) {
            tooltipEl.textContent = tipData.preview.slice(0, 120);
            tooltipEl.style.opacity = '1';
          }
        });

        cy.on('mouseout', 'node', (ev) => {
          const node = ev.target;
          node.style({ 'background-opacity': 0.20, 'border-opacity': 0.7, 'shadow-blur': 6 });
          tooltipEl.style.opacity = '0';
        });

        cy.on('mousemove', 'node', (ev) => {
          const pos = ev.renderedPosition || { x: 0, y: 0 };
          tooltipEl.style.left = (pos.x + 14) + 'px';
          tooltipEl.style.top = (pos.y - 10) + 'px';
        });

        // Click: centrar nodo + desvanecer resto (wineandcheesemap effect)
        cy.on('click', 'node', (ev) => {
          const node = ev.target;
          // Animar vecindario: opacidad plena en nodo + vecinos
          cy.nodes().not(node).not(node.neighborhood()).forEach(n => {
            n.style({ 'opacity': 0.15 });
          });
          cy.edges().forEach(e => {
            e.style({ 'opacity': 0.08 });
          });
          // Vecinos directos opacidad normal
          node.neighborhood().nodes().forEach(n => {
            n.style({ 'opacity': 1 });
          });
          node.style({ 'opacity': 1 });
          // Aristas del vecindario visibles
          node.connectedEdges().forEach(e => {
            e.style({ 'opacity': 0.7 });
          });
          // Centrar
          cy.animate({
            center: { eles: node },
            zoom: 2.2,
            duration: 400,
          });
        });

        // Doble clic: navegar a la página
        cy.on('dblclick', 'node', (ev) => {
          const docId = ev.target.data('doc_id');
          if (onNavigate && docId) onNavigate(docId);
        });

        // Clic en fondo: restaurar todo
        cy.on('click', (ev) => {
          if (ev.target === cy) {
            cy.nodes().forEach(n => n.style({ 'opacity': 1 }));
            cy.edges().forEach(e => e.style({ 'opacity': 0.6 }));
            cy.animate({ zoom: 1, pan: { x: 0, y: 0 }, duration: 300 });
          }
        });

        // Resize al cambiar tamaño del contenedor
        const ro = new ResizeObserver(() => cy.resize().fit(30));
        ro.observe(container);

        // Scroll del contenedor padre: pausar interacción si no visible
        container.__cy = cy;
      })
      .catch(err => {
        console.warn('gioser-graph: error fetching graph:', err);
        container.innerHTML =
          '<div style="padding:1rem;text-align:center;color:rgba(232,234,245,0.35);' +
          'font-size:0.8rem;font-family:Inter,sans-serif;">' +
          '· grafo no disponible ·</div>';
      });
  }

  // Auto-inicializar todos los <gioser-graph> en la página
  function boot() {
    const els = document.querySelectorAll('gioser-graph');
    for (const el of els) {
      // Esperar a que Cytoscape esté cargado
      if (typeof cytoscape !== 'undefined') {
        initGraph(el);
      } else {
        // Si el CDN no ha cargado, esperar
        const check = setInterval(() => {
          if (typeof cytoscape !== 'undefined') {
            clearInterval(check);
            initGraph(el);
          }
        }, 100);
      }
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
})();
