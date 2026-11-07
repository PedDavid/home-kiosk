// Geometry checks for the 1280×800 frame, run in the page. Returns a list of problems (empty when
// the layout fits). It looks at boxes, not pixels, so it holds on any machine.
export function layoutProblems() {
  const problems = [];
  const vw = innerWidth, vh = innerHeight;
  const doc = document.scrollingElement || document.documentElement;
  if (doc.scrollWidth > vw + 1) problems.push(`page scrolls sideways (${doc.scrollWidth}px wide)`);
  if (doc.scrollHeight > vh + 1) problems.push(`page scrolls down (${doc.scrollHeight}px tall)`);
  const stage = document.querySelector('#stage').getBoundingClientRect();
  if (stage.left < -1 || stage.top < -1 || stage.right > vw + 1 || stage.bottom > vh + 1) problems.push('stage outside the viewport');

  // Every visible box in the active view (and any open sheet) must stay inside the stage, unless
  // something between it and the view clips or scrolls it on purpose (slides, lists, ellipses).
  const name = el => el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + (typeof el.className === 'string' && el.className ? '.' + el.className.trim().split(/\s+/).join('.') : '');
  // In ambient/night/off the overlay covers the stage and the view under it isn't seen.
  const mode = document.querySelector('#stage').dataset.mode;
  const overlay = { ambient: '#ambient', night: '#night', off: '#screenoff' }[mode];
  const roots = [overlay || '.view.is-active', '.sheet-layer.is-open .sheet', '.takeover.is-open'].map(s => document.querySelector(s)).filter(Boolean);
  for (const rootEl of roots) {
    const walk = el => {
      for (const child of el.children) {
        const cs = getComputedStyle(child);
        if (cs.display === 'none' || cs.visibility === 'hidden') continue;
        const r = child.getBoundingClientRect();
        if (r.width > 0 && r.height > 0 && cs.position !== 'fixed' &&
          (r.left < stage.left - 1 || r.right > stage.right + 1 || r.top < stage.top - 1 || r.bottom > stage.bottom + 1)) {
          problems.push(`${name(child)} outside the frame (${Math.round(r.left)},${Math.round(r.top)} → ${Math.round(r.right)},${Math.round(r.bottom)})`);
          continue; // report the outermost box only
        }
        if (cs.overflowX === 'visible' && cs.overflowY === 'visible') walk(child);
      }
    };
    walk(rootEl);
  }
  return problems;
}
