# CalDAV recordings

`radicale-scenario.json` holds the requests the CalDAV adapter made in the `scenario` of
`tests/calendar_caldav.rs`, and Radicale 3.9's answers (paths only, no host or credentials).
`replays_the_radicale_recording` replays it with no network.

To check against a real server and record it again:

```sh
python3 -m venv /tmp/rad && /tmp/rad/bin/pip install radicale
printf 'alex:secret-pw\n' > /tmp/rad/users
cat > /tmp/rad/config <<CONF
[server]
hosts = 127.0.0.1:5232
[auth]
type = htpasswd
htpasswd_filename = /tmp/rad/users
htpasswd_encryption = plain
[storage]
filesystem_folder = /tmp/rad/store
[rights]
type = owner_only
CONF
/tmp/rad/bin/radicale --config /tmp/rad/config &
HQ_TEST_RADICALE=http://127.0.0.1:5232 HQ_RECORD=1 \
  cargo test --test calendar_caldav radicale_live -- --ignored
```

The live test also runs the shared `CalendarSource` contract suite against Radicale. Docker
works too (`tomsquest/docker-radicale`) with the same user and rights.
