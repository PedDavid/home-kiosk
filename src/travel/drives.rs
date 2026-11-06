//! The wrapper around any [`TravelTime`] adapter: which events get a drive, caching, the daily
//! budget and several people's origins. Upstream did this in its Worker (`addDrives`); here it's
//! one tested place, so a new adapter only has to answer "how many minutes from A to B".

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use tokio::sync::Mutex;

use super::{Drive, NotConfigured, Place, TravelTime, normalise};
use crate::Clock;

/// Upstream's limits.
pub const AHEAD: Duration = Duration::hours(4);
pub const TTL: Duration = Duration::minutes(10);
pub const DAILY_CAP: u32 = 300;
/// Longer than this is taken as a wrong geocode (another town of the same name), not a drive.
pub const MAX_MINS: u32 = 180;
pub const MAX_EVENTS: usize = 8;
pub const MAX_ORIGINS: usize = 3;

#[derive(Debug, Clone, Copy)]
pub struct Rules {
    /// Only events starting within this from now.
    pub ahead: Duration,
    /// How long a result (or a failure) is kept.
    pub ttl: Duration,
    /// Adapter requests per local day; once spent, no new drives until midnight.
    pub daily_cap: u32,
    pub max_mins: u32,
    /// Events considered per call, soonest first.
    pub max_events: usize,
    /// Distinct origins asked per event.
    pub max_origins: usize,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            ahead: AHEAD,
            ttl: TTL,
            daily_cap: DAILY_CAP,
            max_mins: MAX_MINS,
            max_events: MAX_EVENTS,
            max_origins: MAX_ORIGINS,
        }
    }
}

/// Where drives start: home, and people who have their own origin (id → (name, place)).
#[derive(Debug, Clone, Default)]
pub struct Origins {
    pub home: Option<Place>,
    pub people: BTreeMap<String, (String, Place)>,
}

impl Origins {
    /// `(label, place)` for each of `people`, de-duplicated: a person's own origin labelled with
    /// their name, anyone else home. An event with nobody known starts from home.
    fn for_people(&self, people: &[String], max: usize) -> Vec<(String, &Place)> {
        fn push<'p>(label: &str, place: &'p Place, out: &mut Vec<(String, &'p Place)>) {
            if !out.iter().any(|(_, p)| p.key() == place.key()) {
                out.push((label.to_string(), place));
            }
        }
        let mut out: Vec<(String, &Place)> = Vec::new();
        let mut push_home = false;
        for id in people {
            match self.people.get(id) {
                Some((name, place)) => push(name, place, &mut out),
                None => push_home = true,
            }
        }
        if (push_home || out.is_empty())
            && let Some(home) = &self.home
        {
            push("home", home, &mut out);
        }
        out.truncate(max);
        out
    }
}

/// An event, as far as drives go.
#[derive(Debug, Clone, Copy)]
pub struct Trip<'a> {
    pub id: &'a str,
    pub location: Option<&'a str>,
    pub start: DateTime<Utc>,
    pub all_day: bool,
    pub people: &'a [String],
}

struct Cached {
    at: DateTime<Utc>,
    /// `None`: unknown or failed, kept so it isn't asked again until it expires.
    mins: Option<u32>,
}

#[derive(Default)]
struct State {
    /// (origin key, normalised destination) → result.
    cache: HashMap<(String, String), Cached>,
    day: Option<NaiveDate>,
    used: u32,
}

/// The shared wrapper. Cheap to call on every `/api/calendar` request: answers come from its
/// cache, and the adapter is only asked about what's missing or expired.
pub struct Drives {
    port: Arc<dyn TravelTime>,
    origins: Origins,
    rules: Rules,
    tz: Tz,
    clock: Clock,
    state: Mutex<State>,
}

impl Drives {
    pub fn new(port: Arc<dyn TravelTime>, origins: Origins, rules: Rules, tz: Tz, clock: Clock) -> Self {
        Drives {
            port,
            origins,
            rules,
            tz,
            clock,
            state: Mutex::new(State::default()),
        }
    }

    pub fn not_configured(tz: Tz, clock: Clock) -> Self {
        Self::new(Arc::new(NotConfigured), Origins::default(), Rules::default(), tz, clock)
    }

    /// Whether drives can be computed at all: a provider and at least one origin.
    pub fn configured(&self) -> bool {
        self.port.configured() && (self.origins.home.is_some() || !self.origins.people.is_empty())
    }

    /// Adapter requests made today (local day), for logs and tests.
    pub async fn requests_today(&self) -> u32 {
        let mut st = self.state.lock().await;
        self.roll_day(&mut st, (self.clock)());
        st.used
    }

    fn roll_day(&self, st: &mut State, now: DateTime<Utc>) {
        let today = now.with_timezone(&self.tz).date_naive();
        if st.day != Some(today) {
            st.day = Some(today);
            st.used = 0;
        }
    }

    /// Drives by event id, for the events that qualify and have a known drive. Events that
    /// don't get one fall back to `Travel:` on the frontend.
    ///
    /// - Only timed events with a location, starting after now and within [`Rules::ahead`];
    ///   at most [`Rules::max_events`] of them, soonest first.
    /// - Each origin–destination answer, failures included, is kept for [`Rules::ttl`].
    /// - At most [`Rules::daily_cap`] adapter requests per local day. An event whose origins
    ///   couldn't all be asked gets no drive rather than a possibly short one.
    /// - With several origins (people with their own), the longest drive wins.
    pub async fn for_trips(&self, trips: &[Trip<'_>]) -> HashMap<String, Drive> {
        let mut out = HashMap::new();
        if !self.configured() {
            return out;
        }
        let now = (self.clock)();
        let mut due: Vec<(&Trip, &str)> = trips
            .iter()
            .filter(|t| !t.all_day && t.start > now && t.start - now < self.rules.ahead)
            .filter_map(|t| Some((t, t.location.map(str::trim).filter(|l| !l.is_empty())?)))
            .collect();
        if due.is_empty() {
            return out;
        }
        due.sort_by_key(|(t, _)| t.start);
        due.truncate(self.rules.max_events);

        let mut st = self.state.lock().await;
        self.roll_day(&mut st, now);
        let ttl = self.rules.ttl;
        st.cache.retain(|_, c| now - c.at < ttl);

        for (trip, dest) in due {
            let mut best: Option<Drive> = None;
            let mut complete = true;
            for (label, place) in self.origins.for_people(trip.people, self.rules.max_origins) {
                let key = (place.key(), normalise(dest));
                let mins = match st.cache.get(&key) {
                    Some(c) => c.mins,
                    None if st.used < self.rules.daily_cap => {
                        st.used += 1;
                        let mins = match self.port.minutes(place, dest).await {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!(error = %e, "travel time");
                                None
                            }
                        };
                        st.cache.insert(key, Cached { at: now, mins });
                        mins
                    }
                    None => {
                        complete = false;
                        break;
                    }
                };
                if let Some(m) = mins.filter(|m| *m <= self.rules.max_mins)
                    && best.as_ref().is_none_or(|b| m > b.mins)
                {
                    best = Some(Drive { mins: m, from: label });
                }
            }
            if complete && let Some(b) = best {
                out.insert(trip.id.to_string(), b);
            }
        }
        out
    }

    /// [`for_trips`](Self::for_trips) for calendar events.
    pub async fn for_events(&self, events: &[crate::calendar::Event]) -> HashMap<String, Drive> {
        if !self.configured() {
            return HashMap::new();
        }
        let trips: Vec<Trip> = events.iter().map(super::trip).collect();
        self.for_trips(&trips).await
    }
}
