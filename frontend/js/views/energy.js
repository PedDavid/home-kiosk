// Energy: today so far (electricity per hour from the power sensor), as a sheet opened from the
// Home tab's Energy card or the Home card's energy line. One series, so one colour (ink) and no
// legend; the hour still running is drawn lighter and labelled. Tap a bar for its value.
(function () {
  'use strict';
  var D = window.DATA;
  D.energyToday = null;

  App.fmtWatts = function (w) { return w == null ? '–' : w >= 1000 ? (w / 1000).toFixed(1) + ' kW' : Math.round(w) + ' W'; };
  function kwh(n) { return (n < 10 ? n.toFixed(2) : n.toFixed(1)) + ' kWh'; }
  function hourName(h) { return h === 0 ? '12 AM' : h === 12 ? 'Noon' : (h % 12) + (h < 12 ? ' AM' : ' PM'); }

  App.loadEnergy = function () {
    return App.api('GET', '/home/energy').then(function (e) { D.energyToday = e && !e.none ? e : null; return D.energyToday; });
  };
  // Today's total for the Home card and the Home tab: every 5 minutes (the server keeps 5 too).
  function refresh() { if (D.hasHome) App.loadEnergy().then(function () { App.emit('data'); }, function () {}); }
  App.on('ready', function () { setTimeout(refresh, 3000); setInterval(refresh, 5 * 60e3); });

  function chart(e) {
    var W = 760, H = 200, top = 12, left = 44, base = H - 26, n = 24, slot = (W - left) / n, bar = slot - 2; // 2px gap between bars
    var max = Math.max.apply(null, e.hours.concat([0.1]));
    var step = max > 2 ? 1 : max > 1 ? 0.5 : max > 0.4 ? 0.2 : 0.1, yMax = Math.ceil(max / step) * step;
    var y = function (v) { return base - (v / yMax) * (base - top); };
    var cur = e.hours.length - 1, out = '';
    for (var g = 0; g <= yMax + 1e-9; g += step) { // recessive gridlines, labelled
      out += '<line x1="' + left + '" x2="' + W + '" y1="' + y(g) + '" y2="' + y(g) + '" class="en-grid"/>' +
        '<text x="' + (left - 8) + '" y="' + (y(g) + 4) + '" class="en-ax" text-anchor="end">' + (+g.toFixed(1)) + '</text>';
    }
    for (var h = 0; h < n; h++) {
      var x = left + h * slot + 1;
      if (h % 3 === 0) out += '<text x="' + (x + bar / 2) + '" y="' + (H - 6) + '" class="en-ax" text-anchor="middle">' + hourName(h) + '</text>';
      if (h > cur) continue;
      var v = e.hours[h], hgt = Math.max(v > 0 ? 2 : 0, base - y(v)), r = Math.min(4, hgt / 2, bar / 2);
      // Rounded top, square base: anchored to the baseline.
      var d = 'M' + x + ',' + base + 'V' + (base - hgt + r) + 'q0,-' + r + ' ' + r + ',-' + r + 'H' + (x + bar - r) + 'q' + r + ',0 ' + r + ',' + r + 'V' + base + 'Z';
      out += '<path d="' + d + '" class="en-bar' + (h === cur ? ' is-now' : '') + '"/>' +
        '<rect x="' + (x - 1) + '" y="' + top + '" width="' + slot + '" height="' + (base - top) + '" class="en-hit" data-h="' + h + '"/>';
    }
    out += '<line x1="' + left + '" x2="' + W + '" y1="' + base + '" y2="' + base + '" class="en-base"/>';
    return '<svg class="en-chart" viewBox="0 0 ' + W + ' ' + H + '" role="img" aria-label="Electricity used each hour today, in kilowatt-hours">' + out + '</svg>';
  }

  function sheetHTML(e) {
    if (!e) return '<div class="sheet-pad en"><div class="label">Energy · today so far</div><p class="en-note">No power readings from Home Assistant yet.</p></div>';
    var diff = e.yesterdayByNow > 0 ? Math.round((e.total - e.yesterdayByNow) / e.yesterdayByNow * 100) : null;
    var meterAge = e.meterReported ? Date.now() - e.meterReported : null;
    var when = e.meterReported ? new Date(e.meterReported) : null;
    var meter = !when ? 'The smart meter hasn’t reported usage recently, so gas and costs aren’t shown.'
      : meterAge > 6 * 36e5 ? 'Smart meter last reported ' + App.fmt.rel(when).toLowerCase() + ' ' + App.fmt.time(when) + (meterAge > 864e5 ? ' (' + App.fmt.day(when) + ' ' + when.getDate() + ' ' + App.fmt.monthShort(when) + ')' : '') + ', so its gas and cost figures are out of date.'
      : 'Smart meter last reported at ' + App.fmt.time(when) + '.';
    var cur = e.hours.length - 1;
    return '<div class="sheet-pad en">' +
      '<div class="label">Energy · today so far</div>' +
      '<div class="en-head">' +
      '  <div class="en-hero"><span class="num">' + kwh(e.total) + '</span><small>electricity since midnight' + (e.cost != null ? ' · about £' + e.cost.toFixed(2) : '') + '</small></div>' +
      '  <div class="en-facts">' +
      '    <div><b class="num">' + App.fmtWatts(e.nowW) + '</b><span>now</span></div>' +
      (e.peak ? '    <div><b class="num">' + App.fmtWatts(e.peak.w) + '</b><span>peak, ' + App.fmt.time(new Date(e.peak.at)) + '</span></div>' : '') +
      '    <div><b class="num">' + kwh(e.yesterdayByNow) + '</b><span>yesterday by now' + (diff != null ? ' · ' + (diff > 0 ? '+' : '') + diff + '%' : '') + '</span></div>' +
      '  </div>' +
      '</div>' +
      '<div class="en-plot">' + chart(e) + '</div>' +
      '<div class="en-tip" data-tip>' + hourName(cur) + ' so far · ' + kwh(e.hours[cur] || 0) + ' <span>(tap a bar for its hour)</span></div>' +
      '<p class="en-note">kWh per hour, from the power sensor. The lighter bar is the hour still going. ' + App.esc(meter) + '</p>' +
      '</div>';
  }

  App.energySheet = function () {
    // Shows what's cached at once, then refills with fresh figures.
    App.sheet('<div class="en-wrap">' + sheetHTML(D.energyToday) + '</div>', {
      cls: 'sheet--wide',
      onMount: function (s) {
        App.loadEnergy().then(function (e) { var w = s.querySelector('.en-wrap'); if (w) w.innerHTML = sheetHTML(e); }, function () {});
        s.addEventListener('click', function (ev) {
          var hit = ev.target.closest('[data-h]'), e = D.energyToday;
          if (!hit || !e) return;
          var h = +hit.dataset.h, tip = s.querySelector('[data-tip]');
          s.querySelector('.en-chart').classList.add('has-pick');
          App.$$('.en-bar', s).forEach(function (b, i) { b.classList.toggle('is-picked', i === h); });
          tip.innerHTML = hourName(h) + ' – ' + hourName((h + 1) % 24) + ' · <b>' + kwh(e.hours[h] || 0) + '</b>' + (h === e.hours.length - 1 ? ' so far' : '');
        });
      }
    });
  };
})();
