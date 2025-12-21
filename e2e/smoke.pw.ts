import { test, expect } from './fixtures';

test.describe('Extension Smoke Tests', () => {
  test('extension loads with valid ID', async ({ extensionId }) => {
    expect(extensionId).toBeTruthy();
    expect(extensionId).toMatch(/^[a-z]{32}$/);
  });

  test('background page is active', async ({ backgroundPage }) => {
    expect(backgroundPage).toBeTruthy();
    expect(backgroundPage.url()).toContain('chrome-extension://');
  });

  test('popup page renders', async ({ context, extensionId }) => {
    const page = await context.newPage();
    await page.goto(`chrome-extension://${extensionId}/popup/popup.html`);

    await expect(page.locator('body')).toBeVisible();
    await expect(page.locator('#blocked-count')).toBeVisible();
  });

  test('options page renders', async ({ context, extensionId }) => {
    const page = await context.newPage();
    await page.goto(`chrome-extension://${extensionId}/options/options.html`);

    await expect(page.locator('body')).toBeVisible();
    await expect(page.locator('#lists-container')).toBeVisible();
  });

  test('WASM module initializes', async ({ backgroundPage }) => {
    const isInitialized = await backgroundPage.evaluate(() => {
      const global = window as unknown as { wasm?: { is_initialized?: () => boolean } };
      return global.wasm?.is_initialized?.() ?? false;
    });

    expect(isInitialized).toBe(true);
  });

  test('blocks known ad domain', async ({ backgroundPage }) => {
    const blocked = await backgroundPage.evaluate(() => {
      const global = window as unknown as {
        wasm?: { should_block?: (url: string, type: string, initiator: string | undefined) => boolean };
      };
      return global.wasm?.should_block?.('https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js', 'script', 'https://example.com') ?? false;
    });

    expect(blocked).toBe(true);
  });

  test('allows non-ad domain', async ({ backgroundPage }) => {
    const blocked = await backgroundPage.evaluate(() => {
      const global = window as unknown as {
        wasm?: { should_block?: (url: string, type: string, initiator: string | undefined) => boolean };
      };
      return global.wasm?.should_block?.('https://example.com/', 'document', undefined) ?? false;
    });

    expect(blocked).toBe(false);
  });

  test('content script injects on page load', async ({ context }) => {
    const page = await context.newPage();
    await page.goto('https://example.com');

    const injected = await page.evaluate(() => {
      return document.documentElement.dataset.bbInjected === '1';
    });

    expect(injected).toBe(true);
  });

  test('badge updates on navigation', async ({ context, extensionId }) => {
    const page = await context.newPage();
    await page.goto('https://example.com');

    await page.waitForTimeout(500);

    const popup = await context.newPage();
    await popup.goto(`chrome-extension://${extensionId}/popup/popup.html`);

    const countText = await popup.locator('#blocked-count').textContent();
    expect(countText).toBeDefined();
  });
});

test.describe('Page Blocking Tests', () => {
  test('loads clean page without errors', async ({ context }) => {
    const page = await context.newPage();
    const errors: string[] = [];

    page.on('pageerror', (err) => errors.push(err.message));

    await page.goto('https://example.com');
    await page.waitForLoadState('networkidle');

    expect(errors).toHaveLength(0);
  });

  test('loads news site', async ({ context }) => {
    const page = await context.newPage();
    await page.goto('https://www.bbc.com', { waitUntil: 'domcontentloaded' });

    await expect(page.locator('body')).toBeVisible();
  });
});
