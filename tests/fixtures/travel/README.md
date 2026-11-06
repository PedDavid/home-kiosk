# Travel fixtures

Answers from OSRM (`/route/v1/driving/…`), Photon (`/api?q=…`) and Nominatim
(`/search?q=…&format=jsonv2`), used by `tests/travel.rs` through a fake `Fetch`.

They follow each service's documented response format, with real coordinates for Lisbon and
Sintra. They were written by hand rather than recorded, because the sandbox that wrote them
couldn't reach the public servers. To refresh them from a real deployment:

```sh
curl "$OSRM/route/v1/driving/-9.139337,38.722252;-9.381709,38.802869?overview=false&alternatives=false&steps=false"
curl "$PHOTON/api?q=Sintra&limit=1&lat=38.7223&lon=-9.1393"
curl "$NOMINATIM/search?q=Sintra&format=jsonv2&limit=1"
```
