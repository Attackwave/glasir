// Runs the view's script against stubbed WebGL and 2D contexts.
//
// The page is the one part of this project a Rust self-check cannot reach: a
// typo in the shader setup or a renamed identifier produces a blank screen with
// an error only the browser console sees. This executes layout, clump
// computation, buffer upload and one frame, which is where such mistakes
// surface.
//
// With a second argument, a captured /viewport response is fed through fetch,
// which exercises the streaming path: response -> buffer upload -> frame.
//
// Run: node assets/check_view.js <page.html> [viewport.json]
const fs = require('fs');
const html = fs.readFileSync(process.argv[2] || '/tmp/page.html', 'utf8');
const js = /<script>([\s\S]*)<\/script>/.exec(html)?.[1];
if (!js) { console.error('no script in page'); process.exit(1); }

const gl = new Proxy({
  VERTEX_SHADER: 0, FRAGMENT_SHADER: 1, ARRAY_BUFFER: 2, STATIC_DRAW: 3,
  FLOAT: 4, POINTS: 5, LINES: 6, COMPILE_STATUS: 7, LINK_STATUS: 8,
  COLOR_BUFFER_BIT: 9, BLEND: 10, SRC_ALPHA: 11, ONE_MINUS_SRC_ALPHA: 12,
}, {
  // Every GL entry point returns a truthy object: a stub that returned
  // undefined would fail the shader-compile checks and mask the real error.
  get: (t, k) => k in t ? t[k] : (() => ({})),
});
const ctx2d = new Proxy({}, {get: () => () => {}, set: () => true});
// `withGL` false makes getContext('webgl') return null, which is how the
// fallback path gets exercised. It is also how a real bug was caught: an
// earlier version took a 2D context on the canvas *before* asking for WebGL,
// and a canvas keeps its first context type for life — so WebGL was null in
// every browser, however capable.
const withGL = process.env.NO_WEBGL !== '1';
const el = () => ({
  getContext: t => (t === '2d' ? ctx2d : withGL ? gl : null),
  style: {}, width: 1400, height: 900, dataset: {},
  addEventListener() {}, setAttribute() {}, appendChild() {},
  classList: {add() {}, remove() {}},
  innerHTML: '', textContent: '', onclick: null, oninput: null,
});
global.document = {getElementById: el, querySelectorAll: () => [], createElement: el, documentElement: {}};
global.window = {devicePixelRatio: 1};
global.innerWidth = 1400; global.innerHeight = 900;
global.addEventListener = () => {};
global.requestAnimationFrame = () => {};
global.matchMedia = () => ({matches: false});
global.getComputedStyle = () => ({getPropertyValue: () => '#4bc0b0'});
// Timers fire immediately so the debounced viewport request runs in-process.
global.setTimeout = f => { f(); return 0; };
global.clearTimeout = () => {};
const viewport = process.argv[3] && fs.readFileSync(process.argv[3], 'utf8');
global.fetch = () => viewport
  ? Promise.resolve({json: () => Promise.resolve(JSON.parse(viewport))})
  : Promise.reject(new Error('no viewport fixture'));

// Interaction handlers are captured so states beyond the first frame can be
// exercised: focusing a node took a different path through the renderer and
// its missing helper only surfaced on click, after thousands of console errors.
const handlers = {};
global.addEventListener = (name, fn) => { handlers[name] = fn; };

const label = withGL ? '[webgl] ' : '[canvas] ';

try {
  const api = new Function(js + '\nreturn {setFocus, draw, currentNodes: () => DATA.nodes};')();
  if (viewport) {
    // `fetch` resolves through two `.then` hops, so the response lands a few
    // microtasks in. setImmediate is after all of them.
    setImmediate(() => {
      const nodes = api.currentNodes();
      if (!nodes.length) {
        console.error('view failed: no nodes after viewport response');
        process.exit(1);
      }
      try {
        // Focusing filters the renderer by neighbourhood — a separate branch
        // from the unfocused draw, and where a missing helper hid until click.
        api.setFocus(nodes[0].id);
        api.draw();
        api.setFocus(null);
        api.draw();
      } catch (e) {
        console.error('view failed while focused:', e.message);
        process.exit(1);
      }
      console.log(label + 'view ok: overview, viewport, focus, and frames');
    });
    // The success line for this case is printed above, inside setImmediate.
    return;
  }
  console.log(label + 'view ok: overview and one frame (pass a viewport.json to test streaming)');
} catch (e) {
  console.error('view failed:', e.message);
  process.exit(1);
}
