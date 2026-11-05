// Device codes and issued keys stay on the backend. Only approval details reach this page.
var $ = function (id) { return document.getElementById(id); };
var timer = null, busy = false, generation = 0;
async function request(method, suffix) {
  var opts = { method: method, credentials: 'same-origin', cache: 'no-store' };
  if (method !== 'GET') { opts.headers = { 'content-type': 'application/json' }; opts.body = '{}'; }
  var response = await fetch('/api/immich/auth' + (suffix || ''), opts);
  if (response.status === 401) throw new Error('Unlock this display with its Home HQ key, then try again.');
  var result = await response.json();
  if (!response.ok) throw new Error(result.error || 'Could not reach Immich. Try again.');
  return result;
}
function render(state) {
  clearTimeout(timer);
  $('pending').hidden = state.status !== 'pending';
  $('connect').hidden = ['pending', 'connected', 'disabled'].indexOf(state.status) >= 0;
  $('disconnect').hidden = ['pending', 'connected'].indexOf(state.status) < 0;
  $('disconnect').textContent = state.status === 'pending' ? 'Cancel connection' : 'Disconnect';
  $('connected').hidden = state.status !== 'connected';
  var messages = {
    disabled: 'Device pairing is not enabled on this Home HQ server.',
    disconnected: 'Ready to connect.', expired: 'Access or the approval code expired. Connect again to continue.',
    denied: 'Connection was declined. You can try again.',
    pending: 'Waiting for approval on your phone…',
    connected: 'Connected. Access expires ' + (state.expires_at ? new Date(state.expires_at).toLocaleString() : '') + '.'
  };
  $('status').textContent = messages[state.status] || 'Connection unavailable.';
  if (state.status === 'pending') {
    var link = new URL(state.verification_uri);
    if (link.protocol !== 'https:' && link.protocol !== 'http:') throw new Error('Invalid approval link.');
    $('approval').href = link.href;
    $('code').textContent = state.user_code;
    if (!$('qr').getAttribute('src')) $('qr').src = '/api/immich/auth/qr?t=' + Date.now();
    timer = setTimeout(poll, Math.max(1, state.interval || 5) * 1000);
  } else {
    $('qr').removeAttribute('src');
    if (state.status === 'connected') timer = setTimeout(poll, 30000);
  }
}
async function poll() {
  var current = generation;
  try {
    var state = await request('POST', '/poll');
    if (current !== generation) return;
    $('error').textContent = ''; render(state);
  } catch (error) {
    if (current !== generation) return;
    $('error').textContent = error.message;
    timer = setTimeout(poll, 15000);
  }
}
async function action(method) {
  if (busy) return;
  busy = true; generation++; clearTimeout(timer);
  $('connect').disabled = $('disconnect').disabled = true;
  try { $('error').textContent = ''; render(await request(method)); }
  catch (error) { $('error').textContent = error.message; timer = setTimeout(poll, 15000); }
  finally { busy = false; $('connect').disabled = $('disconnect').disabled = false; }
}
$('connect').addEventListener('click', function () { action('POST'); });
$('disconnect').addEventListener('click', function () { action('DELETE'); });
request('GET').then(render).catch(function (error) { $('error').textContent = error.message; });
