// Regression test for #7: tapping the photo on Home opens that same photo in Photos, both the first
// time and when Photos was already open, and the slideshow carries on from there. No screenshots:
// it checks which photo id each view shows.
import { test, expect, open, NOW } from './kiosk.mjs';

// Id of the photo a slideshow stage shows now (its newest slide, main image).
const showing = (page, stage) => page.evaluate(stage => {
  const img = document.querySelector(stage + ' > .slide:last-child img:not(.blurfill)');
  const m = img && img.getAttribute('src').match(/\/api\/photos\/([^?/]+)/);
  return m ? m[1] : null;
}, stage);
const inPhotos = page => showing(page, '#view-photos .ph-stage');
const onHome = page => showing(page, '#view-home .h-photo-stage');

// Moves the home tile on (12 s per photo) until it shows something other than `id`.
async function homeAwayFrom(page, id) {
  for (let i = 0; i < 20 && (await onHome(page)) === id; i++) await page.clock.runFor(12500);
  expect(await onHome(page), 'test setup: Home should show a different photo than Photos').not.toBe(id);
  return onHome(page);
}

test('home photo opens the same photo in Photos', async ({ page }) => {
  // Fake timers, so the home tile's slideshow can be moved on without waiting.
  await page.clock.install({ time: new Date(NOW) });
  await open(page, 'home');
  // Let the app pre-build the hidden views: Photos' own slideshow has already picked a photo,
  // which is how the bug showed up.
  await page.clock.runFor(5000);
  await expect.poll(() => inPhotos(page)).not.toBeNull();

  // 1. First tap: Photos opens on the photo Home shows (not on the one it had picked itself).
  let id = await homeAwayFrom(page, await inPhotos(page));
  await page.locator('#view-home .h-photo').click();
  await expect(page.locator('#view-photos')).toHaveClass(/is-active/);
  expect(await inPhotos(page), 'Photos should open on the photo tapped on Home').toBe(id);

  // 2. Normal navigation carries on: next shows a photo, previous comes back to the tapped one.
  await page.locator('#view-photos .ph-bottom [data-next]').click();
  expect(await inPhotos(page)).not.toBeNull();
  await page.locator('#view-photos .ph-bottom [data-prev]').click();
  expect(await inPhotos(page), 'previous should return to the tapped photo').toBe(id);
  await page.locator('#view-photos .ph-bottom [data-next]').click();

  // 3. Photos was already open: back to Home, let the tile move on, tap again.
  await page.locator('#rail [data-go="home"]').click();
  await expect(page.locator('#view-home')).toHaveClass(/is-active/);
  id = await homeAwayFrom(page, await inPhotos(page));
  await page.locator('#view-home .h-photo').click();
  await expect(page.locator('#view-photos')).toHaveClass(/is-active/);
  expect(await inPhotos(page), 'reopened Photos should show the photo tapped on Home').toBe(id);

  // 4. Opening Photos from the rail (no photo passed) keeps its own position.
  await page.locator('#rail [data-go="home"]').click();
  await page.locator('#rail [data-go="photos"]').click();
  expect(await inPhotos(page), 'Photos from the rail should stay where it was').toBe(id);
});
