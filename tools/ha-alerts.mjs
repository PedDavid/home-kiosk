// Prints the Home Assistant YAML that pushes urgent alerts to the dashboard the moment they happen:
// a rest_command and an automation for the doorbell, smoke/CO and leak entities in settings.json.
//
//   npm run ha-alerts                   YAML with a placeholder where the token goes
//   npm run ha-alerts -- --with-token   the secrets.yaml line filled in from secrets.json (keep it private)
import { readFileSync } from 'node:fs';

const dir = new URL('..', import.meta.url);
const S = JSON.parse(readFileSync(new URL('settings.json', dir), 'utf8'));
const wrangler = readFileSync(new URL('wrangler.jsonc', dir), 'utf8');
const host = (/"pattern":\s*"([^"]+)"/.exec(wrangler) || [])[1] || '<your dashboard address>';
const al = Object.assign({}, (S.home && S.home.alerts) || {});
if (al.doorbell && al.doorbell.off) delete al.doorbell; // "off": true: not pushed
let token = '<ALERT_TOKEN from secrets.json>';
if (process.argv.includes('--with-token')) token = JSON.parse(readFileSync(new URL('secrets.json', dir), 'utf8')).ALERT_TOKEN || token;
const list = (ids) => ids.map((id) => '          - ' + id).join('\n');

const triggers = [];
if (al.doorbell && al.doorbell.entity) triggers.push('      - trigger: state\n        entity_id: ' + al.doorbell.entity + (/^binary_sensor\./.test(al.doorbell.entity) ? '\n        to: "on"' : '') + '\n        id: doorbell');
if (al.smoke && al.smoke.entities && al.smoke.entities.length) triggers.push('      - trigger: state\n        entity_id:\n' + list(al.smoke.entities) + '\n        to: "on"\n        id: smoke');
if (al.leak && al.leak.entities && al.leak.entities.length) triggers.push('      - trigger: state\n        entity_id:\n' + list(al.leak.entities) + '\n        to: "on"\n        id: leak');
if (!triggers.length) { console.error('No doorbell, smoke or leak alerts in settings.json home.alerts.'); process.exit(1); }

console.log(`# ---- secrets.yaml (Home Assistant config folder) ----
homehq_alert_url: "https://${host}/api/alert?t=${token}"

# ---- configuration.yaml ----
rest_command:
  homehq_alert:
    url: !secret homehq_alert_url
    method: POST
    content_type: application/json
    payload: '{"type": "{{ type }}"}'
    timeout: 5

automation homehq:
  - alias: "Home HQ: push urgent alerts to the frames"
    description: "Tells the wall frames at once, instead of waiting for their next poll."
    mode: queued
    triggers:
${triggers.join('\n')}
    actions:
      - action: rest_command.homehq_alert
        data:
          type: "{{ trigger.id }}"
`);
