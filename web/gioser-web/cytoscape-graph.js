/**
 * gioser-graph.js — Grafo semántico con Cytoscape.js
 *
 * Detecta automáticamente elementos <gioser-graph> agregados al DOM
 * (incluso los creados dinámicamente por el WASM) y monta el grafo.
 *
 * Efecto "wineandcheesemap": clic en nodo → centra + desvanece resto.
 * Doble clic → callback de navegación (window.__gioserGraphNavigate).
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

    // Si cytoscape no ha cargado, esperar
    if (typeof cytoscape === 'undefined') {
      const check = setInterval(() => {
        if (typeof cytoscape !== 'undefined') {
          clearInterval(check);
          initGraph(container);
        }
      }, 100);
      return;
    }

    fetch(apiUrl + '/graph?limit=500')
      .then(function (r) { return r.json(); })
      .then(function (data) {
        if (!data.nodes || data.nodes.length === 0) return;

        var elements = [];

        for (var i = 0; i < data.nodes.length; i++) {
          var d = data.nodes[i].data;
          if (!d.doc_id) continue;
          var color = caminoColor(d.camino);
          elements.push({
            group: 'nodes',
            data: {
              id: d.id,
              doc_id: d.doc_id,
              label: d.name.length > 24 ? d.name.slice(0, 22) + '…' : d.name,
              camino: d.camino.toUpperCase(),
              color: color,
            },
          });
        }

        var nodeIds = {};
        for (var i = 0; i < elements.length; i++) {
          nodeIds[elements[i].data.id] = true;
        }

        for (var i = 0; i < data.edges.length; i++) {
          var ed = data.edges[i].data;
          if (!nodeIds[ed.source] || !nodeIds[ed.target]) continue;
          var weight = ed.weight || 0.7;
          elements.push({
            group: 'edges',
            data: {
              id: ed.id,
              source: ed.source,
              target: ed.target,
              weight: weight,
            },
          });
        }

        // Guardar payload completo para tooltip
        var tipMap = {};
        for (var i = 0; i < data.nodes.length; i++) {
          var d = data.nodes[i].data;
          if (d.doc_id) tipMap[d.id] = d;
        }

        var cy = cytoscape({
          container: container,
          elements: elements,
          style: [
            {
              selector: 'edge',
              style: {
                'width': 'mapData(weight, 0.5, 1.0, 0.5, 4.5)',
                'line-color': 'rgba(255,255,255,0.16)',
                'curve-style': 'haystack',
                'haystack-radius': 0,
                'opacity': 0.6,
              },
            },
            {
              selector: 'node',
              style: {
                'shape': 'round-rectangle',
                'width': 130,
                'height': 34,
                'background-color': 'data(color)',
                'background-opacity': 0.18,
                'border-color': 'data(color)',
                'border-width': 1.8,
                'border-opacity': 0.55,
                'color': 'rgba(232,234,245,0.88)',
                'font-size': 11,
                'font-family': 'Inter, system-ui, sans-serif',
                'font-weight': 500,
                'text-valign': 'center',
                'text-halign': 'center',
                'label': 'data(label)',
                'min-zoomed-font-size': 8,
                'transition-property': 'background-opacity, border-opacity, border-width',
                'transition-duration': 180,
              },
            },
          ],
          layout: {
            name: 'cose',
            animate: false,
            idealEdgeLength: 150,
            nodeRepulsion: 7000,
            gravity: 0.2,
            numIter: 800,
            fit: true,
            padding: 25,
          },
        });

        // Tooltip
        var tooltipEl = document.createElement('div');
        tooltipEl.className = 'cy-tooltip';
        tooltipEl.style.cssText =
          'position:absolute;z-index:10;pointer-events:none;' +
          'background:rgba(6,5,13,0.90);color:#e8eaf5;' +
          'padding:6px 10px;border-radius:8px;font-size:11px;' +
          'font-family:Inter,sans-serif;line-height:1.4;' +
          'border:1px solid rgba(255,255,255,0.10);' +
          'backdrop-filter:blur(8px);max-width:220px;' +
          'opacity:0;transition:opacity 180ms ease;';
        container.style.position = 'relative';
        container.appendChild(tooltipEl);

        cy.on('mouseover', 'node', function (ev) {
          var n = ev.target;
          n.style({ 'background-opacity': 0.45, 'border-opacity': 0.9, 'border-width': 2.2 });
          var tipData = tipMap[n.id()];
          if (tipData && tipData.preview) {
            tooltipEl.textContent = tipData.preview.slice(0, 130);
            tooltipEl.style.opacity = '1';
          }
        });

        cy.on('mouseout', 'node', function (ev) {
          var n = ev.target;
          n.style({ 'background-opacity': 0.18, 'border-opacity': 0.55, 'border-width': 1.8 });
          tooltipEl.style.opacity = '0';
        });

        cy.on('mousemove', function (ev) {
          if (tooltipEl.style.opacity === '1') {
            var pos = ev.renderedPosition || { x: 0, y: 0 };
            tooltipEl.style.left = (pos.x + 14) + 'px';
            tooltipEl.style.top = (pos.y - 10) + 'px';
          }
        });

        // Click nodo: centrar + desvanecer resto
        cy.on('click', 'node', function (ev) {
          var node = ev.target;
          // Vecinos
          cy.nodes().not(node).not(node.neighborhood()).forEach(function (n) {
            n.style({ 'opacity': 0.12 });
          });
          cy.edges().forEach(function (e) {
            e.style({ 'opacity': 0.06 });
          });
          node.neighborhood().nodes().forEach(function (n) {
            n.style({ 'opacity': 1 });
          });
          node.style({ 'opacity': 1, 'background-opacity': 0.40, 'border-opacity': 1 });
          node.connectedEdges().forEach(function (e) {
            e.style({ 'opacity': 0.7 });
          });
          cy.animate({
            center: { eles: node },
            zoom: 2.5,
            duration: 350,
          });
        });

        // Doble clic: callback de navegación
        cy.on('dblclick', 'node', function (ev) {
          var docId = ev.target.data('doc_id');
          if (onNavigate && docId) onNavigate(docId);
        });

        // Clic en fondo: restaurar todo
        cy.on('click', function (ev) {
          if (ev.target === cy) {
            cy.nodes().forEach(function (n) {
              n.style({ 'opacity': 1, 'background-opacity': 0.18, 'border-opacity': 0.55, 'border-width': 1.8 });
            });
            cy.edges().forEach(function (e) {
              e.style({ 'opacity': 0.6 });
            });
            cy.animate({ zoom: 1, pan: { x: 0, y: 0 }, duration: 300 });
          }
        });

        // ResizeObserver para redimensionar con el contenedor
        var ro = new ResizeObserver(function () {
          cy.resize().fit(25);
        });
        ro.observe(container);

        // Scroll del deck: pausar interacciones del grafo
        var deckEl = container.closest('.deck');
        if (deckEl) {
          deckEl.addEventListener('scroll', function () {
            // No hacemos nada especial, el grafo se redimensiona solo
          });
        }
      })
      .catch(function (err) {
        console.warn('gioser-graph: error:', err);
        container.innerHTML =
          '<div style="padding:1rem;text-align:center;color:rgba(232,234,245,0.30);' +
          'font-size:0.8rem;font-family:Inter,sans-serif;">' +
          '· grafo no disponible ·</div>';
      });
  }

  // MutationObserver: detecta <gioser-graph> agregados en cualquier momento
  var observer = new MutationObserver(function (mutations) {
    for (var m = 0; m < mutations.length; m++) {
      var added = mutations[m].addedNodes;
      for (var i = 0; i < added.length; i++) {
        var el = added[i];
        if (el.tagName && el.tagName.toLowerCase() === 'gioser-graph') {
          initGraph(el);
        }
        // También revisar hijos
        var graphs = el.querySelectorAll ? el.querySelectorAll('gioser-graph') : [];
        for (var j = 0; j < graphs.length; j++) {
          initGraph(graphs[j]);
        }
      }
    }
  });

  observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
  });

  // También inicializar los que ya existen (si el DOM ya está listo)
  var existing = document.querySelectorAll('gioser-graph');
  for (var i = 0; i < existing.length; i++) {
    initGraph(existing[i]);
  }
})();
