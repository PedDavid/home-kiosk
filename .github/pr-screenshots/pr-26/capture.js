// Captures the wall view against the live backend (serving fixture ICS feeds).
const { chromium } = require('playwright');
const fs = require('fs'), path = require('path');
const root = '/home/user/home-kiosk/frontend', out = process.argv[2];
const NOW = '2026-11-05T17:30:00Z';
function googleFontsCss() {
  const pkg = n => path.join(root, 'node_modules', n);
  const css = (n, f, fam) => fs.readFileSync(path.join(pkg(n), f), 'utf8')
    .replace(/font-family: '[^']+'/g, `font-family: '${fam}'`)
    .replace(/url\(\.\/files\//g, `url(https://fonts.gstatic.com/fontsource/${n}/`);
  return [css('@fontsource-variable/archivo', 'wdth.css', 'Archivo'),
    css('@fontsource/instrument-serif', '400.css', 'Instrument Serif'),
    css('@fontsource/instrument-serif', '400-italic.css', 'Instrument Serif')].join('\n');
}
(async () => {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, timezoneId: 'Europe/Lisbon', hasTouch: true, reducedMotion: 'reduce' });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.clock.setFixedTime(new Date(NOW));
  await page.route('https://fonts.googleapis.com/**', r => r.fulfill({ status: 200, contentType: 'text/css', body: googleFontsCss() }));
  await page.route('https://fonts.gstatic.com/**', r => {
    const m = new URL(r.request().url()).pathname.match(/^\/fontsource\/(.+)\/([^/]+\.woff2)$/);
    const f = m && path.join(root, 'node_modules', m[1], 'files', m[2]);
    return f && fs.existsSync(f) ? r.fulfill({ status: 200, contentType: 'font/woff2', body: fs.readFileSync(f) }) : r.abort();
  });
  const settle = async () => {
    await page.waitForFunction(() => window.App && App.current && document.querySelector('#railClock b'));
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(600);
  };
  const shot = async name => { await page.screenshot({ path: path.join(out, name + '.png') }); console.log('wrote', name); };
  await page.goto('http://127.0.0.1:8080/#home');
  await settle();
  await shot('home-next-up');
  await page.goto('http://127.0.0.1:8080/#calendar');
  await settle();
  for (const [mode, name] of [['day', 'calendar-today'], ['3day', 'calendar-3day'], ['week', 'calendar-week'], ['month', 'calendar-month']]) {
    await page.click(`button[data-mode="${mode}"]`);
    await page.waitForTimeout(400);
    await shot(name);
  }
  console.log('page errors:', errors);
  await browser.close();
})();
