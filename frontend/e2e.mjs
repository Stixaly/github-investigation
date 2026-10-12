import { readFile, mkdir } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';

const output = process.env.SCREENSHOT_DIR || '/output';
await mkdir(output, { recursive: true });
const password = (await readFile(process.env.PASSWORD_FILE || '/bootstrap_password', 'utf8')).trim();
// Alpine provides Mesa rather than SwiftShader; these flags apply only to this test browser.
const browser = await chromium.launch({
  executablePath: '/usr/bin/chromium',
  env: { ...process.env, LIBGL_ALWAYS_SOFTWARE: '1', EGL_PLATFORM: 'surfaceless' },
  args: [
    '--no-sandbox',
    '--disable-dev-shm-usage',
    '--use-gl=angle',
    '--use-angle=gl-egl',
    '--ignore-gpu-blocklist',
  ],
});
const context = await browser.newContext({ viewport: { width: 1440, height: 1050 }, locale: 'fr-FR' });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.stack || error.message));
try {
  await page.goto(process.env.BASE_URL || 'http://localhost:8088');
  await page.getByLabel('Identifiant', { exact: true }).fill(process.env.LOGIN || 'analyste');
  await page.getByLabel('Mot de passe', { exact: true }).fill(password);
  await page.getByRole('button', { name: 'Ouvrir mon espace' }).click();
  await page.getByRole('button', { name: 'Terrain de démonstration Responsable' }).click();
  await page.locator('.graph canvas').first().waitFor();
  await page
    .locator('.graph[data-engine="3d"][data-ready="true"][data-simulation="idle"]')
    .waitFor({ timeout: 60000 });
  await page.screenshot({ path: output + '/graphe-3d-desktop.png', fullPage: true });
  const canvas3d = page.locator('.graph canvas').first();
  const beforeOrbit = await canvas3d.evaluate((canvas) => canvas.toDataURL());
  const bounds = await canvas3d.boundingBox();
  await page.mouse.move(bounds.x + bounds.width * 0.8, bounds.y + bounds.height * 0.65);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width * 0.55, bounds.y + bounds.height * 0.48, { steps: 12 });
  await page.mouse.up();
  assert.notEqual(await canvas3d.evaluate((canvas) => canvas.toDataURL()), beforeOrbit);
  await page.getByLabel('Rechercher dans le graphe').fill('demo-alice');
  await page
    .locator('.graph-search-results')
    .getByRole('button', { name: 'demo-alice', exact: true })
    .click();
  await page.getByRole('button', { name: 'Fermer le détail' }).click();
  const centered = await canvas3d.boundingBox();
  await page.mouse.move(centered.x + centered.width / 2, centered.y + centered.height / 2);
  await page.waitForFunction(
    () => document.querySelector('.float-tooltip-kap')?.textContent === 'demo-alice',
  );
  await canvas3d.click({ position: { x: centered.width / 2, y: centered.height / 2 } });
  await page.waitForFunction(() => document.querySelector('.inspector h3')?.textContent === 'demo-alice');
  assert.equal(await page.locator('.inspector h3').innerText(), 'demo-alice');
  await page.getByRole('button', { name: 'Consulter la preuve' }).click();
  await page.locator('.proof').waitFor();
  await page.getByRole('button', { name: 'Isoler le voisinage sélectionné' }).click();
  await page.waitForFunction(
    () => Number(document.querySelector('.graph-stats')?.getAttribute('data-nodes')) === 3,
  );
  await page.getByRole('button', { name: 'Isoler le voisinage sélectionné' }).click();
  await page.getByRole('button', { name: 'Fermer le détail' }).click();
  const threeDownload = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Exporter la vue en PNG' }).click();
  assert.equal(await (await threeDownload).failure(), null);
  for (const layout of ['circle', 'breadthfirst', 'grid', 'cose']) {
    await page.getByLabel('Disposition du graphe').selectOption(layout);
    await page.locator('.graph[data-simulation="idle"]').waitFor({ timeout: 60000 });
  }
  await page.getByRole('button', { name: '2D', exact: true }).click();
  await page.locator('.graph[data-engine="2d"][data-ready="true"]').waitFor();
  await page.locator('tbody tr').first().waitFor();
  assert((await page.locator('tbody tr').count()) >= 5);
  await page.waitForFunction(
    () =>
      Number(document.querySelector('.graph-stats')?.getAttribute('data-nodes')) ===
      document.querySelectorAll('tbody tr').length,
  );
  const visibleCount = async () => Number(await page.locator('.graph-stats').getAttribute('data-nodes'));
  const total = await visibleCount();
  const repositories = page.locator('.graph-filters').getByRole('button', { name: /Dépôts/ });
  await repositories.click();
  await page.waitForFunction(
    (total) => Number(document.querySelector('.graph-stats')?.getAttribute('data-nodes')) < total,
    total,
  );
  await repositories.click();
  await page.waitForFunction(
    (total) => Number(document.querySelector('.graph-stats')?.getAttribute('data-nodes')) === total,
    total,
  );
  await page.getByLabel('Rechercher dans le graphe').fill('demo-alice');
  await page
    .locator('.graph-search-results')
    .getByRole('button', { name: 'demo-alice', exact: true })
    .click();
  assert.equal(await page.locator('.inspector h3').innerText(), 'demo-alice');
  await page.getByRole('button', { name: 'Isoler le voisinage sélectionné' }).click();
  await page.waitForFunction(
    (total) => Number(document.querySelector('.graph-stats')?.getAttribute('data-nodes')) < total,
    total,
  );
  assert((await visibleCount()) > 1);
  await page.getByRole('button', { name: 'Isoler le voisinage sélectionné' }).click();
  await page.getByRole('button', { name: 'Fermer le détail' }).click();
  await page.getByRole('button', { name: 'Libellés des relations' }).click();
  assert.equal(
    await page.getByRole('button', { name: 'Libellés des relations' }).getAttribute('aria-pressed'),
    'false',
  );
  await page.getByRole('button', { name: 'Libellés des relations' }).click();
  for (const layout of ['circle', 'breadthfirst', 'grid', 'cose'])
    await page.getByLabel('Disposition du graphe').selectOption(layout);
  const zoomBefore = Number((await page.getByLabel('Niveau de zoom').innerText()).replace('%', ''));
  await page.getByRole('button', { name: 'Zoom avant', exact: true }).click();
  assert(Number((await page.getByLabel('Niveau de zoom').innerText()).replace('%', '')) > zoomBefore);
  await page.getByRole('button', { name: 'Cadrer le graphe' }).click();
  await page.getByRole('button', { name: 'Agrandir le graphe' }).click();
  await page.getByRole('button', { name: 'Panneau des preuves' }).click();
  assert(await page.locator('.inspector').isHidden());
  await page.screenshot({ path: output + '/graphe-atelier.png', fullPage: true });
  const downloaded = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Exporter la vue en PNG' }).click();
  const download = await downloaded;
  assert.equal(download.suggestedFilename(), 'investigation-graphe.png');
  assert.equal(await download.failure(), null);
  await page.getByRole('button', { name: 'Panneau des preuves' }).click();
  await page.keyboard.press('Escape');
  assert(!(await page.locator('.explorer').getAttribute('class')).includes('graph-expanded'));
  await page.screenshot({ path: output + '/exploration-desktop.png', fullPage: true });
  // Use the canvas engine's coordinates, then perform a real pointer click on a relation.
  const edgePoint = await page.evaluate(() => {
    const cy = document.querySelector('.graph')._cyreg.cy;
    const edge = cy
      .edges()
      .filter((e) => e.data('label') === 'auteur de')
      .first();
    cy.fit(edge.union(edge.connectedNodes()), 100);
    // The force layout is not deterministic: the midpoint can sit under another
    // node. Take the first point of the segment that no node body covers.
    const start = edge.source().renderedPosition();
    const end = edge.target().renderedPosition();
    const bodies = cy.nodes(':visible').map((node) => node.renderedBoundingBox({ includeLabels: false }));
    const free = (point) =>
      !bodies.some((box) => point.x >= box.x1 && point.x <= box.x2 && point.y >= box.y1 && point.y <= box.y2);
    for (const share of [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8]) {
      const point = { x: start.x + (end.x - start.x) * share, y: start.y + (end.y - start.y) * share };
      if (free(point)) return point;
    }
    return edge.renderedMidpoint();
  });
  await page.locator('.graph').click({ position: edgePoint });
  assert.equal(await page.locator('.inspector h3').innerText(), 'AUTHORED');
  await page.getByRole('button', { name: 'Consulter la preuve' }).click();
  await page.locator('.proof').waitFor();
  await page.locator('tbody tr button').first().click();
  await page.getByRole('button', { name: 'Consulter la preuve' }).click();
  await page.locator('.proof').waitFor();
  assert((await page.locator('.proof').innerText()).includes('payload'));
  await page.getByRole('button', { name: 'Scans & chronologie', exact: true }).click();
  await page.getByRole('button', { name: 'Comparer au précédent' }).first().click();
  await page.locator('.comparison').waitFor();
  await page.screenshot({ path: output + '/chronologie-desktop.png', fullPage: true });
  await page.getByRole('button', { name: 'Exploration', exact: true }).click();
  await page.locator('.graph[data-ready="true"]').waitFor();
  // Whole-investigation view, common neighbours and cross-investigation search.
  await page.getByLabel('Scan affiché').selectOption('all');
  await page.locator('.graph[data-ready="true"]').waitFor();
  await page.waitForFunction(() => document.querySelectorAll('tbody tr').length === 14);
  for (const label of ['demo-alice', 'demo-bob']) {
    await page.getByLabel('Rechercher dans le graphe').fill(label);
    await page.locator('.graph-search-results').getByRole('button', { name: label, exact: true }).click();
    if (label === 'demo-alice') await page.getByRole('button', { name: 'Retenir pour comparer' }).click();
  }
  await page.getByRole('button', { name: 'Voisins communs avec demo-alice' }).click();
  await page.getByRole('heading', { name: '1 voisin commun' }).waitFor();
  await page.getByRole('button', { name: 'Fermer le détail' }).click();
  await page.getByLabel('Rechercher dans tous les dossiers').fill('alice');
  await page.locator('.search-result', { hasText: 'demo-alice' }).first().waitFor();
  await page.getByLabel('Rechercher dans tous les dossiers').fill('');
  // A discovery plan is shown without calling GitHub; nothing is launched here.
  await page.getByText('Découvrir sur GitHub').click();
  await page.getByLabel('Requête GitHub').fill('loader');
  await page.getByRole('button', { name: 'Voir le plan' }).click();
  await page.getByLabel('Plan de la découverte').getByText('loader fork:true').waitFor();
  await page.getByText('Découvrir sur GitHub').click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.locator('.graph[data-engine="3d"][data-simulation="idle"]').waitFor({ timeout: 60000 });
  await page.screenshot({ path: output + '/exploration-mobile.png', fullPage: true });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
  const fallback = await page.context().newPage();
  await fallback.addInitScript(() => {
    const original = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (type, ...args) {
      return type === 'webgl' || type === 'webgl2' ? null : original.call(this, type, ...args);
    };
  });
  await fallback.goto(process.env.BASE_URL || 'http://localhost:8088');
  await fallback.getByRole('button', { name: 'Terrain de démonstration Responsable' }).click();
  await fallback.locator('.graph[data-engine="2d"][data-ready="true"]').waitFor();
  assert((await fallback.locator('.graph-renderer-notice').innerText()).includes('indisponible'));
  await fallback.close();
  await page.getByRole('button', { name: 'Déconnexion', exact: true }).click();
  await page.getByRole('button', { name: 'Ouvrir mon espace' }).waitFor();
  assert.deepEqual(errors, []);
  console.log(
    'Browser checks passed: 3D rendering, orbit, node click and proof, neighborhood, PNG, 2D switch, filters, layouts, zoom, relation click, timeline, whole-investigation view, common neighbours, search, discovery plan, mobile, WebGL fallback, logout.',
  );
} catch (error) {
  console.log(
    'Graph diagnostic:',
    await page
      .locator('.graph-stats')
      .textContent()
      .catch(() => ''),
  );
  console.log('Browser errors:', errors);
  console.log(
    'Canvas state:',
    await page
      .locator('.graph')
      .evaluate((e) => ({ dataset: { ...e.dataset }, html: e.innerHTML.slice(0, 2000) })),
  );
  await page.screenshot({ path: output + '/browser-failure.png', fullPage: true });
  throw error;
} finally {
  await browser.close();
}
