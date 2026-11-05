// Shared state. Starts empty with the right shapes; js/sync.js fills it from the API (and from the
// last copy saved on this device, so the wall display comes up instantly and survives outages).
window.DATA = {
  people: [], ALL: [],
  family: { id: 'family', name: 'Family', color: '#111111', rgb: '17 17 17', ink: '#FFFFFF' },
  events: [],
  stores: [{ id: 'grocer', name: 'Groceries' }], catOrder: { all: ['Other'], grocer: ['Other'] },
  groceries: [], catalog: [], usuals: [],
  hasHome: false, homeOnline: false, homeError: null,
  rooms: [], devices: [], climate: null, media: null, cameras: [], scenes: [],
  sensors: {}, alertConfig: {}, energy: null, checks: null, awayOffMinutes: 0,
  weather: null, warnings: [],
  bins: null,
  photos: []
};
