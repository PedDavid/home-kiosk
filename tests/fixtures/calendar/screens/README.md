Feeds for looking at the calendar on the frame with real ICS data (not used by tests): a
household of Alex and Sam around Thursday 5 November 2026. Serve them and point the backend at
them, e.g.

    (cd tests/fixtures/calendar/screens && python3 -m http.server 8099 --bind 127.0.0.1) &
    HQ_CALENDARS='[{"url":"http://127.0.0.1:8099/alex.ics","people":["alex"]},
                   {"url":"http://127.0.0.1:8099/sam.ics","people":"sam"},
                   {"url":"http://127.0.0.1:8099/family.ics","people":"all"}]' cargo run

and pin the browser's clock to 2026-11-05T17:30:00Z (Europe/Lisbon).
