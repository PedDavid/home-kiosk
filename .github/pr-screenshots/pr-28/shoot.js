const { chromium } = require('playwright');
(async () => {
  const out = process.argv[2];
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, timezoneId: process.argv[3] || 'Asia/Tokyo' });
  const p = await ctx.newPage();
  await p.goto('http://127.0.0.1:18080/#home');
  await p.waitForFunction(() => window.App && App.current && document.querySelector('#railClock b'));
  await p.waitForTimeout(1500);
  await p.screenshot({ path: out + '/home-leave-by.png' });
  await p.goto('http://127.0.0.1:18080/#calendar');
  await p.waitForTimeout(1500);
  const evs = await p.$$eval('[data-ev]', els => els.map(e => [e.dataset.ev, e.textContent.slice(0, 60)]));
  console.log(JSON.stringify(evs));
  await p.screenshot({ path: out + '/calendar.png' });
  for (const [id, name] of [['Lunch with Gran', 'sheet-drive'], ['Haircut', 'sheet-travel-fallback']]) {
    await p.locator('[data-ev]', { hasText: id }).last().click();
    await p.waitForTimeout(800);
    await p.screenshot({ path: `${out}/${name}.png` });
    console.log(name, await p.locator('.evs-leave').first().innerText().catch(() => 'none'));
    await p.keyboard.press('Escape'); await p.waitForTimeout(400);
    await p.goto('http://127.0.0.1:18080/#calendar'); await p.waitForTimeout(1000);
  }
  await b.close();
})();
