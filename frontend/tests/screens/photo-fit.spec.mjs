import { test, expect, open, settle } from './kiosk.mjs';

for (const viewport of [{ width: 1280, height: 800 }, { width: 800, height: 1100 }]) {
  test(`photo fit and Polaroid bounds at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.route('**/api/photos/fit-*', route => {
      const portrait = route.request().url().includes('portrait');
      const w = portrait ? 900 : 4000, h = portrait ? 2400 : 1000;
      return route.fulfill({ contentType: 'image/svg+xml', body:
        `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><path fill="#297b91" d="M0 0h${w}v${h}H0z"/><path fill="#e95342" d="M0 0h${w * .08}v${h}H0z"/><path fill="#49ae66" d="M${w * .92} 0h${w * .08}v${h}H${w * .92}z"/><path fill="#f5ce60" d="M0 0h${w}v${h * .04}H0z"/><path fill="#bd6fe2" d="M0 ${h * .96}h${w}v${h * .04}H0z"/></svg>` });
    });
    await open(page, 'photos');
    await page.evaluate(() => {
      App.setIdle({ home: 3600, rest: 3600 });
      // Render known slides rather than depending on the production shuffle/queue.
      const area = document.createElement('div');
      area.id = 'fit-fixture'; area.className = 'ph-stage';
      document.querySelector('#view-photos .ph-stage').replaceWith(area);
      window.fitFixture = new App.Slideshow(area, { captions: true });
    });
    for (const layout of ['cover', 'pair', 'collage', 'polaroid']) {
      for (const portrait of [false, true]) {
        const photo = { id: `fit-${portrait ? 'portrait' : 'landscape'}`, o: portrait ? 'p' : 'l', date: '2024-06-12', place: 'Fixture', tag: '2024-06-12' };
        const photos = Array.from({ length: layout === 'collage' ? 3 : layout === 'pair' ? 2 : 1 }, () => photo);
        for (const fullscreen of [false, true]) {
          await page.evaluate(({ layout, photos, fullscreen }) => {
            App.setPhotoFullscreen(fullscreen);
            fitFixture.render({ layout, photos });
          }, { layout, photos, fullscreen });
          await settle(page);
          const failures = await page.evaluate(() => {
            const errors = [];
            const area = document.querySelector('#fit-fixture').getBoundingClientRect();
            for (const im of document.querySelectorAll('#fit-fixture img:not(.blurfill)')) {
              if (getComputedStyle(im).objectFit !== 'contain') errors.push('Photo is cropped instead of fitted');
              const parent = im.closest('.polaroid, .pane, .kb').getBoundingClientRect();
              if (parent.left < area.left - 1 || parent.right > area.right + 1 || parent.top < area.top - 1 || parent.bottom > area.bottom + 1) errors.push('Photo/frame is clipped by the slide');
            }
            const frame = document.querySelector('#fit-fixture .polaroid');
            if (App.photoFullscreen && frame && parseFloat(getComputedStyle(frame).paddingBottom) <= 0) errors.push('Polaroid loses its bottom border without a caption');
            return errors;
          });
          expect(failures, `${layout}, portrait=${portrait}, fullscreen=${fullscreen}`).toEqual([]);
          if (fullscreen && layout === 'polaroid' && portrait) {
            await expect(page).toHaveScreenshot(`photos-polaroid-fullscreen-${viewport.width}.png`);
          }
        }
      }
    }
  });
}
