// Share target for the phone app (m.webmanifest). Android's Share → Home HQ posts the photos
// here; they wait in a cache and /upload?shared picks them up. Handling the post on the phone
// keeps the upload signed in: the dashboard cookie is SameSite=Lax and may not ride along on a
// share from another app. Every other request goes to the network as usual.
self.addEventListener('install', function () { self.skipWaiting(); });
self.addEventListener('activate', function (e) { e.waitUntil(self.clients.claim()); });

self.addEventListener('fetch', function (e) {
  var url = new URL(e.request.url);
  if (e.request.method !== 'POST' || url.pathname !== '/share') return;
  e.respondWith((async function () {
    var form = await e.request.formData();
    var files = form.getAll('photos').filter(function (f) { return f && f.type && f.type.indexOf('image/') === 0; });
    await caches.delete('hq-share');
    var cache = await caches.open('hq-share');
    await Promise.all(files.map(function (f, i) {
      return cache.put('/shared/' + i, new Response(f, { headers: {
        'content-type': f.type, 'x-name': encodeURIComponent(f.name || 'photo.jpg'), 'x-modified': String(f.lastModified || '')
      } }));
    }));
    return Response.redirect('/upload?shared=' + files.length, 303);
  })());
});
