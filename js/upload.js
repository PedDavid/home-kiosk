// Phone upload page: shrink each photo in the browser (full 1920 px + small 960 px JPEGs, rotated
// upright), read the date it was taken from EXIF, and send both to the Worker.
var $ = function (s) { return document.querySelector(s); };
var people = [];

function api(method, path, body) {
  return fetch('/api' + path, { method: method, body: body, credentials: 'same-origin' }).then(function (r) {
    if (r.status === 401) { $('#locked').hidden = false; $('#app').hidden = true; throw new Error('locked'); }
    return r.json().then(function (j) { if (!r.ok) throw new Error(j.error || 'HTTP ' + r.status); return j; });
  });
}

// DateTimeOriginal (or DateTime) from a JPEG's EXIF block → 'YYYY-MM-DD', or null.
function exifDate(buf) {
  var v = new DataView(buf);
  if (v.byteLength < 4 || v.getUint16(0) !== 0xFFD8) return null;
  for (var off = 2; off + 4 < v.byteLength;) {
    var marker = v.getUint16(off), size = v.getUint16(off + 2);
    if (marker === 0xFFE1 && v.getUint32(off + 4) === 0x45786966) {
      var t = off + 10, le = v.getUint16(t) === 0x4949;
      var u16 = function (o) { return v.getUint16(t + o, le); }, u32 = function (o) { return v.getUint32(t + o, le); };
      var readIfd = function (o, want) {
        var n = u16(o), found = {};
        for (var i = 0; i < n; i++) {
          var e = o + 2 + i * 12, tag = u16(e);
          if (want.indexOf(tag) > -1) found[tag] = u16(e + 2) === 2 ? String.fromCharCode.apply(null, new Uint8Array(buf, t + u32(e + 8), 19)) : u32(e + 8);
        }
        return found;
      };
      var ifd0 = readIfd(u32(4), [0x0132, 0x8769]);
      var exif = ifd0[0x8769] ? readIfd(ifd0[0x8769], [0x9003]) : {};
      var s = exif[0x9003] || ifd0[0x0132];
      var m = s && /^(\d{4}):(\d{2}):(\d{2})/.exec(s);
      return m && m[1] !== '0000' ? m[1] + '-' + m[2] + '-' + m[3] : null;
    }
    if ((marker & 0xFF00) !== 0xFF00 || marker === 0xFFDA) break;
    off += 2 + size;
  }
  return null;
}

function localYmd(d) { return d.getFullYear() + '-' + ('0' + (d.getMonth() + 1)).slice(-2) + '-' + ('0' + d.getDate()).slice(-2); }

function decode(file) {
  // createImageBitmap applies the EXIF orientation, so portrait phone photos come out upright.
  if (window.createImageBitmap) return createImageBitmap(file).catch(function () { return viaImg(file); });
  return viaImg(file);
}
function viaImg(file) {
  return new Promise(function (res, rej) {
    var img = new Image(), url = URL.createObjectURL(file);
    img.onload = function () { URL.revokeObjectURL(url); res(img); };
    img.onerror = function () { URL.revokeObjectURL(url); rej(new Error('Not an image this browser can read')); };
    img.src = url;
  });
}
function shrink(bmp, max, quality) {
  var w = bmp.width, h = bmp.height, k = Math.min(1, max / Math.max(w, h));
  var c = document.createElement('canvas');
  c.width = Math.round(w * k); c.height = Math.round(h * k);
  c.getContext('2d').drawImage(bmp, 0, 0, c.width, c.height);
  return new Promise(function (res) { c.toBlob(res, 'image/jpeg', quality); });
}

function upload(file, li) {
  var date;
  return file.slice(0, 256 * 1024).arrayBuffer().then(function (head) {
    try { date = exifDate(head); } catch (e) { date = null; } // odd or truncated EXIF: fall back below
    date = date || localYmd(new Date(file.lastModified || Date.now()));
    return decode(file);
  }).then(function (bmp) {
    var o = bmp.height > bmp.width ? 'p' : 'l';
    return Promise.all([shrink(bmp, 1920, 0.86), shrink(bmp, 960, 0.8)]).then(function (out) {
      var form = new FormData();
      form.append('full', out[0], 'full.jpg');
      form.append('small', out[1], 'small.jpg');
      form.append('meta', JSON.stringify({ o: o, date: date, place: $('#place').value.trim(), from: $('#from').value }));
      return api('POST', '/photos', form);
    });
  }).then(function () {
    li.lastChild.textContent = 'Added · ' + date;
    li.lastChild.className = 'ok';
  }, function (err) {
    li.lastChild.textContent = err.message;
    li.lastChild.className = 'err';
  });
}

function list() {
  return api('GET', '/state').then(function (s) {
    people = s.people;
    var from = $('#from'), keep = from.value || localStorage.getItem('hq-from') || '';
    from.innerHTML = '<option value="">—</option>' + people.map(function (p) { return '<option value="' + p.id + '">' + p.name + '</option>'; }).join('');
    from.value = keep;
    $('#count').textContent = s.photos.length;
    $('#grid').innerHTML = s.photos.map(function (p) {
      return '<div class="ph"><img loading="lazy" src="/api/photos/' + p.id + '?s=1" alt=""><span>' + p.date + (p.place ? ' · ' + p.place.replace(/</g, '&lt;') : '') + '</span>' +
        '<button data-del="' + p.id + '" aria-label="Delete">×</button></div>';
    }).join('');
    $('#app').hidden = false;
  });
}

$('#from').addEventListener('change', function () { try { localStorage.setItem('hq-from', this.value); } catch (e) {} });
function addFiles(files) {
  var chain = Promise.resolve();
  files.forEach(function (f) {
    var li = document.createElement('li');
    li.innerHTML = '<span></span><span>Waiting…</span>';
    li.firstChild.textContent = f.name;
    $('#queue').appendChild(li);
    chain = chain.then(function () { li.lastChild.textContent = 'Uploading…'; return upload(f, li); });
  });
  return chain.then(list);
}
$('#files').addEventListener('change', function () {
  var files = Array.prototype.slice.call(this.files);
  this.value = '';
  addFiles(files);
});

// ------------------------------------------------------------ shared from the phone's gallery
// Share → Home HQ posts the photos to /share; public/share-sw.js keeps them in a cache and opens
// this page with ?shared. They wait for "Upload" so "Where" can be filled in first.
function sharedFiles() {
  if (!window.caches) return Promise.resolve([]);
  return caches.open('hq-share').then(function (c) {
    return c.keys().then(function (keys) {
      return Promise.all(keys.map(function (k) {
        return c.match(k).then(function (r) {
          return r.blob().then(function (b) {
            return new File([b], decodeURIComponent(r.headers.get('x-name') || 'photo.jpg'), { type: b.type, lastModified: +r.headers.get('x-modified') || Date.now() });
          });
        });
      }));
    });
  });
}
function clearShared() { return window.caches ? caches.delete('hq-share') : Promise.resolve(); }
if (/[?&]shared/.test(location.search)) {
  sharedFiles().then(function (files) {
    var box = $('#shared');
    box.hidden = false;
    if (!files.length) {
      box.querySelector('span').textContent = 'The shared photos didn’t come through. Choose them below instead.';
      box.querySelector('button').hidden = true;
      return;
    }
    box.querySelector('span').textContent = files.length + (files.length === 1 ? ' shared photo' : ' shared photos') + ' ready.';
    box.querySelector('button').onclick = function () {
      box.hidden = true;
      history.replaceState(null, '', '/upload');
      addFiles(files).then(clearShared);
    };
  });
}
if ('serviceWorker' in navigator) navigator.serviceWorker.register('/share-sw.js').catch(function () {});
$('#grid').addEventListener('click', function (e) {
  var b = e.target.closest('[data-del]');
  if (!b || !confirm('Delete this photo from the frame?')) return;
  api('DELETE', '/photos/' + b.dataset.del).then(list, function (err) { alert(err.message); });
});
list().catch(function () {});
