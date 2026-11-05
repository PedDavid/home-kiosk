// ICS feeds → flat event list for the dashboard. Recurring events are expanded inside the window;
// moved or cancelled occurrences (RECURRENCE-ID overrides) and EXDATEs are respected.
import ICAL from 'ical.js';

const MAX_PER_SERIES = 1000;

function ymd(t) {
  return t.year + '-' + String(t.month).padStart(2, '0') + '-' + String(t.day).padStart(2, '0');
}

// "Travel: 15", "travel 15", "Travel time - 15 min" anywhere in the description becomes a leave-by
// reminder; it's the fallback when there's no live drive time (see addDrives in index.js).
// "Walk: 5" is added to a live drive time: parking and getting from the car to the door.
// Google keeps formatted descriptions as HTML, so tags and entities are removed first.
function plainText(desc) {
  return (desc || '').replace(/<br\s*\/?>/gi, '\n').replace(/<[^>]+>/g, '')
    .replace(/&nbsp;/g, ' ').replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>');
}
function travelFrom(desc) {
  const m = /travel(?:\s*time)?\s*[:\-\u2013=]?\s*(\d{1,3})/i.exec(plainText(desc));
  return m ? +m[1] : undefined;
}
function walkFrom(desc) {
  const m = /walk(?:ing)?(?:\s*time)?\s*[:\-\u2013=]?\s*(\d{1,3})/i.exec(plainText(desc));
  return m ? +m[1] : undefined;
}

function toEvent(id, details, people) {
  const s = details.startDate, e = details.endDate || details.startDate, item = details.item;
  const desc = plainText(item.description).trim();
  const ev = { id, title: item.summary || '(busy)', people };
  if (s.isDate) {
    ev.allDay = true;
    ev.start = ymd(s);
    ev.end = ymd(e.compare(s) > 0 ? e : s.clone().adjust(1, 0, 0, 0));
  } else {
    ev.start = s.toJSDate().toISOString();
    ev.end = e.toJSDate().toISOString();
  }
  if (item.location) ev.loc = item.location;
  if (desc) ev.notes = desc.length > 280 ? desc.slice(0, 277) + '…' : desc;
  const travel = travelFrom(desc);
  if (travel) ev.travel = travel;
  const walk = walkFrom(desc);
  if (walk) ev.walk = walk;
  ev.key = (item.uid || id) + '|' + ev.start; // used to merge one invite seen on two calendars
  return ev;
}

export function parseIcs(text, people, fromMs, toMs, idPrefix) {
  const root = new ICAL.Component(ICAL.parse(text));
  root.getAllSubcomponents('vtimezone').forEach((tz) => ICAL.TimezoneService.register(tz));

  const from = ICAL.Time.fromJSDate(new Date(fromMs), true);
  const to = ICAL.Time.fromJSDate(new Date(toMs), true);
  const masters = new Map(), exceptions = [];
  for (const vevent of root.getAllSubcomponents('vevent')) {
    const ev = new ICAL.Event(vevent);
    if (ev.isRecurrenceException()) exceptions.push(ev);
    else masters.set(ev.uid, ev);
  }
  for (const ex of exceptions) {
    const m = masters.get(ex.uid);
    if (m) m.relateException(ex);
    else masters.set(ex.uid + '#' + ex.recurrenceId, ex);
  }

  const out = [];
  let n = 0;
  for (const ev of masters.values()) {
    const status = (ev.component.getFirstPropertyValue('status') || '').toUpperCase();
    if (status === 'CANCELLED') continue;
    if (!ev.isRecurring()) {
      const end = ev.endDate || ev.startDate;
      if (end.compare(from) < 0 || ev.startDate.compare(to) > 0) continue;
      out.push(toEvent(idPrefix + (n++), { startDate: ev.startDate, endDate: ev.endDate, item: ev }, people));
      continue;
    }
    const it = ev.iterator();
    for (let i = 0, next; i < MAX_PER_SERIES && (next = it.next()); i++) {
      if (next.compare(to) > 0) break;
      const d = ev.getOccurrenceDetails(next);
      if (d.endDate.compare(from) < 0) continue;
      const st = (d.item.component.getFirstPropertyValue('status') || '').toUpperCase();
      if (st === 'CANCELLED') continue;
      out.push(toEvent(idPrefix + (n++), d, people));
    }
  }
  return out;
}
