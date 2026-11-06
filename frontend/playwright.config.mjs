// Screenshot tests for the wall view (tests/screens/). See tests/screens/README.md.
// `npm run test:screens` checks them, `npm run test:screens:update` rewrites the baselines; both
// build first. Baselines are only comparable when rendered in the same image as CI
// (mcr.microsoft.com/playwright at this @playwright/test version): use the :docker variants
// to run them on any machine with Docker.
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'tests/screens',
  testMatch: '*.spec.mjs',
  // Baselines sit next to the tests, one PNG per screen: tests/screens/screenshots/<name>.png.
  snapshotPathTemplate: '{testDir}/screenshots/{arg}{ext}',
  outputDir: 'test-results/screens',
  reporter: process.env.CI ? [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]] : 'list',
  forbidOnly: !!process.env.CI,
  retries: 0,
  fullyParallel: true,
  use: {
    // The PF1007L: 1280×800 touchscreen, 1:1 pixels, in the household's timezone.
    viewport: { width: 1280, height: 800 },
    screen: { width: 1280, height: 800 },
    deviceScaleFactor: 1,
    hasTouch: true,
    locale: 'en-GB',
    timezoneId: 'Europe/Lisbon',
    reducedMotion: 'reduce',
    serviceWorkers: 'block',
    ...(process.env.CHROMIUM_PATH ? { launchOptions: { executablePath: process.env.CHROMIUM_PATH } } : {}),
  },
  expect: {
    toHaveScreenshot: { animations: 'disabled', caret: 'hide', scale: 'css' },
  },
  projects: [{ name: 'chromium' }],
});
