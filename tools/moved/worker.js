// Stand-in for the dashboard's old address after a rename (see wrangler.jsonc beside this file).
// Every page redirects to the new address. A visitor who proves they have the key (?key= or the
// old cookie, checked against the old Worker's DASHBOARD_KEY secret) gets it added to the
// redirect, so the new address signs them in without typing. Pages still open at the old address
// see a new build in /build.json and reload into the redirect; their API calls get 410 meanwhile.
const enc = new TextEncoder();
function sameKey(a, b) {
  const x = enc.encode(a), y = enc.encode(b);
  return x.byteLength === y.byteLength && crypto.subtle.timingSafeEqual(x, y);
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url), to = new URL(env.NEW_ORIGIN);
    if (url.pathname === '/build.json') return Response.json({ build: 'moved' }, { headers: { 'cache-control': 'no-store' } });
    if (url.pathname.indexOf('/api/') === 0) return Response.json({ error: 'The dashboard moved to ' + to.host }, { status: 410 });

    const dest = new URL(url.pathname + url.search, to);
    const m = /(?:^|;\s*)hq=([^;]+)/.exec(req.headers.get('cookie') || '');
    const key = url.searchParams.get('key') || (m ? decodeURIComponent(m[1]) : '');
    dest.searchParams.delete('key');
    if (key && env.DASHBOARD_KEY && sameKey(key, env.DASHBOARD_KEY)) dest.searchParams.set('key', key);
    // 302, not 301: browsers would cache a permanent redirect, key and all.
    return new Response(null, { status: 302, headers: { location: dest.toString(), 'cache-control': 'no-store' } });
  },
};
