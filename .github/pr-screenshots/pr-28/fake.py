# Serves an ICS feed relative to now, and Photon/OSRM answers from tests/fixtures/travel.
import http.server, datetime, sys, os, urllib.parse
FIX = sys.argv[1]
def ics():
    now = datetime.datetime.now(datetime.timezone.utc).replace(second=0, microsecond=0)
    f = lambda d: d.strftime('%Y%m%dT%H%M%SZ')
    ev = lambda uid, title, start, mins, loc, desc: f"BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:{f(now)}\r\nDTSTART:{f(start)}\r\nDTEND:{f(start+datetime.timedelta(minutes=mins))}\r\nSUMMARY:{title}\r\nLOCATION:{loc}\r\nDESCRIPTION:{desc}\r\nEND:VEVENT\r\n"
    a = now + datetime.timedelta(minutes=50)
    b = now + datetime.timedelta(minutes=150)
    return ("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//shots//EN\r\n" +
            ev("sintra@shots", "Lunch with Gran", a, 90, "Sintra", "Travel: 40") +
            ev("barber@shots", "Haircut", b, 45, "Barbearia do Bairro", "Travel: 20") +
            "END:VCALENDAR\r\n")
class H(http.server.BaseHTTPRequestHandler):
    def send(self, code, body, ctype='application/json'):
        b = body.encode() if isinstance(body, str) else body
        self.send_response(code); self.send_header('content-type', ctype); self.send_header('content-length', str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        u = urllib.parse.urlparse(self.path); q = urllib.parse.parse_qs(u.query)
        if u.path == '/cal.ics': return self.send(200, ics(), 'text/calendar')
        if u.path == '/api':
            name = 'photon-sintra.json' if 'sintra' in q.get('q', [''])[0].lower() else 'photon-empty.json'
            return self.send(200, open(os.path.join(FIX, name)).read())
        if u.path.startswith('/route/v1/'): return self.send(200, open(os.path.join(FIX, 'osrm-route-ok.json')).read())
        self.send(404, '{}')
    def log_message(self, *a): sys.stderr.write(self.path + "\n")
http.server.ThreadingHTTPServer(('127.0.0.1', 18999), H).serve_forever()
