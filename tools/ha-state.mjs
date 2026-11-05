// Prints the Home Assistant YAML that pushes the states the dashboard shows to the Worker, on every
// change and once a minute: a rest_command whose template builds the payload inside Home Assistant,
// and an automation. The Worker then reads a few KB instead of Home Assistant's whole state list,
// which cost most of its CPU. Without it the Worker falls back to reading Home Assistant itself.
//
//   npm run ha-state                   YAML with a placeholder where the token goes
//   npm run ha-state -- --with-token   the secrets.yaml line filled in from secrets.json (keep it private)
//
// Uses the same ALERT_TOKEN as `npm run ha-alerts`; if that's already in Home Assistant's
// secrets.yaml you only need the new homehq_state_url line.
import { readFileSync } from 'node:fs';

const dir = new URL('..', import.meta.url);
const S = JSON.parse(readFileSync(new URL('settings.json', dir), 'utf8'));
const wrangler = readFileSync(new URL('wrangler.jsonc', dir), 'utf8');
const host = (/"pattern":\s*"([^"]+)"/.exec(wrangler) || [])[1] || '<your dashboard address>';
const H = S.home;
if (!H) { console.error('No home section in settings.json.'); process.exit(1); }
const al = Object.assign({}, H.alerts || {}), E = H.energy || {};
if (al.doorbell && al.doorbell.off) delete al.doorbell; // "off": true: not pushed
let token = '<ALERT_TOKEN from secrets.json>';
if (process.argv.includes('--with-token')) token = JSON.parse(readFileSync(new URL('secrets.json', dir), 'utf8').toString()).ALERT_TOKEN || token;

// The same entities the Worker keeps (coreIds in worker/index.js), split into the ones whose change
// should push at once and the busy ones (power, costs) that ride along with the once-a-minute push.
const quiet = new Set(['zone.home']);
(H.rooms || []).forEach((r) => (r.devices || []).forEach((d) => quiet.add(d.entity)));
[H.climate, al.doorbell && al.doorbell.entity, al.doorbell && al.doorbell.lock, al.leak && al.leak.valve].forEach((id) => { if (id) quiet.add(id); });
[].concat((al.leak && al.leak.entities) || [], (al.smoke && al.smoke.entities) || []).forEach((id) => quiet.add(id));
(S.people || []).forEach((p) => { if (p.ha) quiet.add(p.ha); });
const busy = new Set([E.power].concat(E.cost || [], E.usage || []).filter(Boolean));
const all = Array.from(new Set([...quiet, ...busy]));
// The attributes worker/index.js reads; keep in sync with it.
const KEEP = ['device_class', 'friendly_name', 'brightness', 'supported_color_modes', 'current_position', 'temperature',
  'target_temp_step', 'target_temp_low', 'min_temp', 'max_temp', 'hvac_modes', 'current_temperature', 'current_humidity',
  'latitude', 'longitude', 'entity_picture', 'volume_level', 'is_volume_muted', 'media_title', 'media_artist',
  'media_album_name', 'media_content_id', 'app_name', 'supported_features', 'unit_of_measurement'];
const q = (a) => a.map((id) => "'" + id + "'").join(', ');

console.log(`# ---- secrets.yaml (Home Assistant config folder) ----
homehq_state_url: "https://${host}/api/ha-push?t=${token}"

# ---- configuration.yaml ----
rest_command:
  homehq_state:
    url: !secret homehq_state_url
    method: POST
    content_type: application/json
    timeout: 10
    # Built here, in Home Assistant: the entities the dashboard shows, every media player, and every
    # battery sensor (the Worker keeps only the alert sensors' batteries), each with only the attributes
    # the dashboard reads (a few attributes are dates, which can't be written as JSON).
    payload: >-
      {% set keep = [${KEEP.map((k) => "'" + k + "'").join(', ')}] %}
      {% set ns = namespace(l=[]) %}
      {% for s in expand([${q(all)}]) + (states.media_player | list) + (states.sensor | selectattr('attributes.device_class', 'defined') | selectattr('attributes.device_class', 'eq', 'battery') | list) %}
      {% set p = namespace(a=[]) %}
      {% for k in keep %}{% if k in s.attributes %}{% set p.a = p.a + [[k, s.attributes[k]]] %}{% endif %}{% endfor %}
      {% set ns.l = ns.l + [{'entity_id': s.entity_id, 'state': s.state, 'attributes': dict(p.a), 'last_changed': s.last_changed.isoformat()}] %}
      {% endfor %}
      {{ {'states': ns.l} | to_json }}

automation homehq_state:
  - alias: "Home HQ: push the dashboard's states"
    description: "Sends the Worker the states the dashboard shows, on every change and once a minute, so it never has to read Home Assistant's whole state list."
    mode: queued
    max: 5
    triggers:
      - trigger: state
        entity_id:
${[...quiet].map((id) => '          - ' + id).join('\n')}
      - trigger: time_pattern
        minutes: "/1"
    actions:
      - action: rest_command.homehq_state
      - delay: "00:00:02"
`);
